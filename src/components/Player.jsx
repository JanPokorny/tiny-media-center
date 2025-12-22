import { useState, useRef, useEffect } from "react";
import { useKeyboard } from "../hooks/useKeyboard";

const formatDuration = (seconds) => {
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  return `${String(h).padStart(2, '0')}:${String(m).padStart(2, '0')}`;
};

function Player({ movie, onBack }) {
  const videoRef = useRef(null);
  const [isPaused, setIsPaused] = useState(false);
  const [displayTime, setDisplayTime] = useState('');
  const [clockTime, setClockTime] = useState(new Date());

  const skipStats = useRef({ count: 0, lastTime: 0 });

  const MENU_OPTIONS = ["unpause"]; // Keeping this for potential future expansion

  useEffect(() => {
    const restoreProgress = () => {
      const progressSeconds = videoRef.current.duration * parseFloat(localStorage.getItem(`video:${movie.fullPath}:progress`));
      if (Number.isNaN(progressSeconds)) return;
      videoRef.current.removeEventListener("playing", restoreProgress);
      if (progressSeconds > videoRef.current.duration - 10) return;
      videoRef.current.currentTime = Math.floor(progressSeconds);
    }
    videoRef.current?.addEventListener("playing", restoreProgress);
  }, [videoRef])

  useEffect(() => {
    const timer = setInterval(() => setClockTime(new Date()), 1000);
    return () => clearInterval(timer);
  }, []);

  useEffect(() => {
    const video = videoRef.current;
    if (!video || !video.duration) return;
    const remaining = video.duration - video.currentTime;
    const end = new Date(Date.now() + remaining * 1000);
    const currentFormatted = clockTime.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', hour12: false });
    const remainingFormatted = formatDuration(remaining);
    const endFormatted = end.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', hour12: false });
    setDisplayTime(`now ${currentFormatted} + remains ${remainingFormatted} = ends at ${endFormatted}`);
  }, [clockTime]);

  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;
    const handleEnded = () => onBack();
    video.addEventListener("ended", handleEnded);
    return () => video.removeEventListener("ended", handleEnded);
  }, [onBack])

  useEffect(() => {
    const progressInterval = setInterval(() => saveCurrentProgress(), 60000);
    return () => clearInterval(progressInterval);
  }, []);

  const saveCurrentProgress = () => {
    const video = videoRef?.current;
    if (!video || video.duration <= 0) return;
    localStorage.setItem(`video:${movie.fullPath}:progress`, (video.currentTime / video.duration).toString());
  };

  const handleSkip = (direction) => {
    const video = videoRef.current;
    if (!video || !video.duration) return;

    const now = Date.now();
    const stats = skipStats.current;

    if (now - stats.lastTime > 1000) {
      stats.count = 0;
    }

    stats.count += 1;
    stats.lastTime = now;

    const skipAmount = stats.count >= 6 ? 60 : 10;

    const newTime = Math.max(
      0,
      Math.min(video.duration, video.currentTime + direction * skipAmount)
    );
    video.currentTime = newTime;
  };

  useKeyboard({
    Enter: () => {
      if (isPaused) {
        // Since "unpause" is the only option, directly unpause
        setIsPaused(false);
      } else {
        setIsPaused(true);
        // setMenuIndex(0);
      }
    },
    ArrowUp: () => {
      // With only one option, these keys effectively do nothing
      // if (isPaused) setMenuIndex((prev) => Math.max(0, prev - 1));
    },
    ArrowDown: () => {
      // if (isPaused) setMenuIndex((prev) => Math.min(MENU_OPTIONS.length - 1, prev + 1));
    },
    ArrowLeft: () => {
      if (!isPaused) handleSkip(-1);
    },
    ArrowRight: () => {
      if (!isPaused) handleSkip(1);
    },
    Escape: () => {
      if (isPaused) setIsPaused(false);
      else {
        saveCurrentProgress();
        onBack();
      }
    },
    BrowserBack: () => {
      if (isPaused) setIsPaused(false);
      else {
        saveCurrentProgress();
        onBack();
      }
    },
    " ": () => setIsPaused((prev) => !prev),
  });

  useEffect(() => {
    if (isPaused) {
      videoRef.current?.pause();
    } else {
      videoRef.current?.play();
    }
  }, [isPaused]);

  return (
    <div className="player-container">
      <video
        ref={videoRef}
        src={`file://${movie.fullPath}`}
        className="video-player"
        autoPlay
      />
      {isPaused && (
        <div className="pause-overlay">
          <header>
            <span>{movie.name}</span>
          </header>

          <div className="pause-content">
            {/* Simplified display for the single "unpause" option */}
            <div className="pause-item selected">
                {`> ${MENU_OPTIONS[0]}`}
            </div>
          </div>

          <footer>
            <span>{displayTime}</span>
          </footer>
        </div>
      )}
    </div>
  );
}

export default Player;
