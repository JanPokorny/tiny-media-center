import React, { useState, useRef, useEffect } from 'react'
import Header from './Header'
import Footer from './Footer'
import { useKeyboard } from '../hooks/useKeyboard'

function Player({ movie, onBack }) {
  const videoRef = useRef(null)
  const [isPaused, setIsPaused] = useState(false)
  
  // Use refs for high-frequency input tracking to avoid render thrashing
  const skipStats = useRef({ count: 0, lastTime: 0 })
  
  // Pause Menu State
  const [menuIndex, setMenuIndex] = useState(0)
  const MENU_OPTIONS = ['unpause', 'audio [en]', 'subtitles [off]', 'go to time']
  
  // Metadata for footer
  const [watchedPct, setWatchedPct] = useState(0)
  const [endsTime, setEndsTime] = useState('')

  useEffect(() => {
    const video = videoRef.current
    if (!video) return

    const handleTimeUpdate = () => {
      // Check for valid duration to avoid NaN
      if (video.duration) {
        const pct = (video.currentTime / video.duration) * 100
        setWatchedPct(Math.floor(pct))
        
        const remaining = video.duration - video.currentTime
        const end = new Date(Date.now() + remaining * 1000)
        setEndsTime(end.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }))
      }
    }

    const handleEnded = () => onBack()

    video.addEventListener('timeupdate', handleTimeUpdate)
    video.addEventListener('ended', handleEnded)
    
    return () => {
      video.removeEventListener('timeupdate', handleTimeUpdate)
      video.removeEventListener('ended', handleEnded)
      // Save progress on exit
      if (video.duration) {
        window.ipcRenderer.invoke('save-metadata', movie.path, { 
          watchedPercentage: video.currentTime / video.duration,
          lastTime: video.currentTime
        })
      }
    }
  }, [movie.path, onBack])

  const handleSkip = (direction) => {
    const video = videoRef.current
    if (!video || !video.duration) return

    const now = Date.now()
    const stats = skipStats.current
    
    // Reset count if more than 1s has passed
    if (now - stats.lastTime > 1000) {
      stats.count = 0
    }
    
    stats.count += 1
    stats.lastTime = now

    const skipAmount = stats.count >= 6 ? 60 : 10
    
    // Clamp time to valid range
    const newTime = Math.max(0, Math.min(video.duration, video.currentTime + (direction * skipAmount)))
    video.currentTime = newTime
  }

  // Keyboard handling changes based on paused state
  useKeyboard({
    Enter: () => {
      if (isPaused) {
        if (menuIndex === 0) setIsPaused(false) // unpause
        // Other options implementation TBD
      } else {
        setIsPaused(true)
        setMenuIndex(0) // Reset to 'unpause'
      }
    },
    ArrowUp: () => {
      if (isPaused) setMenuIndex(prev => Math.max(0, prev - 1))
    },
    ArrowDown: () => {
      if (isPaused) setMenuIndex(prev => Math.min(MENU_OPTIONS.length - 1, prev + 1))
    },
    ArrowLeft: () => {
      if (!isPaused) handleSkip(-1)
    },
    ArrowRight: () => {
      if (!isPaused) handleSkip(1)
    },
    Backspace: () => {
      if (isPaused) setIsPaused(false)
      else onBack()
    },
    Escape: () => {
      if (isPaused) setIsPaused(false)
      else onBack()
    },
    BrowserBack: () => {
      if (isPaused) setIsPaused(false)
      else onBack()
    },
    ' ': () => setIsPaused(prev => !prev)
  })

  useEffect(() => {
    if (isPaused) {
      videoRef.current?.pause()
    } else {
      videoRef.current?.play()
    }
  }, [isPaused])

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
          {/* Header shows Title in pause mode per image */}
          <div className="pause-header">
            {movie.name}
          </div>
          <div className="header-clock" style={{position: 'absolute', top: '20px', right: '30px', fontSize: '2.5rem', color: '#666'}}>
             {new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}
          </div>

          <div className="pause-content">
            <div className="pause-menu-list">
              {MENU_OPTIONS.map((option, index) => (
                <div 
                  key={option} 
                  className={`pause-item ${index === menuIndex ? 'selected' : ''}`}
                >
                  {index === menuIndex ? `> ${option}` : `\u00A0\u00A0${option}`}
                </div>
              ))}
            </div>
          </div>

          <Footer 
            leftText={`watched ${watchedPct}%`} 
            rightText={`ends ${endsTime}`} 
          />
        </div>
      )}
    </div>
  )
}

export default Player