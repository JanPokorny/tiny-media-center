import { useState, useRef, useEffect, useMemo, useCallback } from "react";
import { useKeyboard } from "../hooks/useKeyboard";
import Menu from "./Menu";

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
  const [activeAudioTrackLabel, setActiveAudioTrackLabel] = useState('unknown');
  const [nativeAudioTracks, setNativeAudioTracks] = useState([]);

  const skipStats = useRef({ count: 0, lastTime: 0 });

  const [menu, setMenu] = useState('pause');
  const [selectedPauseselectedIndex, setPauseSelectedIndex] = useState(0);
  const [selectedAudioStreamIndex, setSelectedAudioStreamIndex] = useState(0); // Index for the menu, not the actual audio track index

  const menuItems = useMemo(() => [
    { key: 'resume', label: 'resume' },
    { key: 'audio', label: `audio [${activeAudioTrackLabel}]` },
  ], [activeAudioTrackLabel]);

  const handleAudioTracksChange = useCallback(() => {
    const video = videoRef.current;
    if (!video || !video.audioTracks) return;

    const tracks = Array.from(video.audioTracks);
    setNativeAudioTracks(tracks.map((track, idx) => ({
      ...track,
      key: track.id || `native-track-${idx}`,
      label: track.label || track.language || `track ${idx + 1}`
    })));

    const activeTrack = tracks.find(track => track.enabled);
    if (activeTrack) {
      const nativeActiveTrackIndex = tracks.indexOf(activeTrack);
      setActiveAudioTrackLabel(activeTrack.label || activeTrack.language || `track ${nativeActiveTrackIndex + 1}`);
      setSelectedAudioStreamIndex(nativeActiveTrackIndex);
      localStorage.setItem(`video:${movie.fullPath}:audioTrackIndex`, nativeActiveTrackIndex.toString());
    }
  }, [videoRef.current, movie.fullPath]);

  useEffect(() => {
    const video = videoRef.current;
    if (!video) return;

    const handleLoadedMetadata = () => {
      const savedAudioTrackIndex = parseInt(localStorage.getItem(`video:${movie.fullPath}:audioTrackIndex`), 10);
      if (!isNaN(savedAudioTrackIndex) && savedAudioTrackIndex < video.audioTracks.length) {
        Array.from(video.audioTracks).forEach((track, index) => {
          track.enabled = index === savedAudioTrackIndex;
        });
      }

      video.audioTracks.addEventListener('change', handleAudioTracksChange);
      handleAudioTracksChange(); // Initial call
    };

    video.addEventListener('loadedmetadata', handleLoadedMetadata);

    return () => {
      video.removeEventListener('loadedmetadata', handleLoadedMetadata);
      if (video.audioTracks) {
        video.audioTracks.removeEventListener('change', handleAudioTracksChange);
      }
    };
  }, [videoRef.current, handleAudioTracksChange, movie.fullPath]);

  useEffect(() => {
    const restoreProgress = () => {
      const progressSeconds = videoRef.current.duration * parseFloat(localStorage.getItem(`video:${movie.fullPath}:progress`));
      if (Number.isNaN(progressSeconds)) return;
      videoRef.current.removeEventListener("playing", restoreProgress);
      if (progressSeconds > videoRef.current.duration - 10) return;
      videoRef.current.currentTime = Math.floor(progressSeconds);
    }
    videoRef.current?.addEventListener("playing", restoreProgress);
  }, [videoRef, movie.fullPath])

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
  }, [clockTime, videoRef.current?.currentTime]);

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

  useEffect(() => {
    videoRef.current.play();
  }, []);

  const handleSelect = useCallback(async (item, index) => {
    if (menu === 'pause') {
      switch (item.key) {
        case 'resume':
          setIsPaused(false);
          break;
        case 'audio':
          setMenu('audio');
          break;
      }
    } else if (menu === 'audio') {
      const video = videoRef.current;
      if (video && video.audioTracks) {
        const tracks = Array.from(video.audioTracks);
        if (index >= 0 && index < tracks.length) {
          tracks.forEach((track, i) => {
            track.enabled = (i === index);
          });
          // Manually trigger the update to ensure the UI reflects the change immediately
          handleAudioTracksChange();
          setMenu('pause');
        }
      }
    }
  }, [menu, onBack, handleAudioTracksChange]);

  useKeyboard({
    Enter: () => {
      if (isPaused) {
        const currentMenuItems = menu === 'pause' ? menuItems : nativeAudioTracks; // Use nativeAudioTracks for the audio menu
        const currentSelectedIndex = menu === 'pause' ? selectedPauseselectedIndex : selectedAudioStreamIndex;
        handleSelect(currentMenuItems[currentSelectedIndex], currentSelectedIndex);
      } else {
        setIsPaused(true);
      }
    },
    ArrowUp: () => {
      if (isPaused) {
        if (menu === 'pause') {
          setPauseSelectedIndex((prev) => Math.max(0, prev - 1));
        } else {
          setSelectedAudioStreamIndex((prev) => Math.max(0, prev - 1));
        }
      }
    },
    ArrowDown: () => {
      if (isPaused) {
        if (menu === 'pause') {
          setPauseSelectedIndex((prev) => Math.min(menuItems.length - 1, prev + 1));
        } else {
          setSelectedAudioStreamIndex((prev) => Math.min(nativeAudioTracks.length - 1, prev + 1)); // Use nativeAudioTracks length
        }
      }
    },
    ArrowLeft: () => {
      if (!isPaused) handleSkip(-1);
    },
    ArrowRight: () => {
      if (!isPaused) handleSkip(1);
    },
    Escape: () => {
      if (isPaused) {
        if (menu === 'audio') {
          setMenu('pause');
        } else {
          setIsPaused(false);
        }
      }
      else {
        saveCurrentProgress();
        onBack();
      }
    },
    BrowserBack: () => {
      if (isPaused) {
        if (menu === 'audio') {
          setMenu('pause');
        } else {
          setIsPaused(false);
        }
      }
      else {
        saveCurrentProgress();
        onBack();
      }
    },
    " ": () => setIsPaused((prev) => !prev),
  }, [isPaused, menu, selectedPauseselectedIndex, selectedAudioStreamIndex, nativeAudioTracks, menuItems, handleSelect]);

  useEffect(() => {
    if (isPaused) {
      videoRef.current?.pause();
    } else {
      videoRef.current?.play();
    }
  }, [isPaused]);

  let currentMenu;
  if (menu === 'pause') {
    currentMenu = <Menu items={menuItems} title={movie.name} selectedIndex={selectedPauseselectedIndex} />;
  } else if (menu === 'audio') {
    currentMenu = <Menu items={nativeAudioTracks} title="Select Audio Stream" selectedIndex={selectedAudioStreamIndex} />;
  }

  return (
    <div className="player-container">
      <video
        ref={videoRef}
        src={`file://${movie.fullPath}`}
        className="video-player"
      />
      {isPaused && (
        <div className="pause-overlay">
          {currentMenu}
          <footer>
            <span>{displayTime}</span>
          </footer>
        </div>
      )}
    </div>
  );
}

export default Player;

