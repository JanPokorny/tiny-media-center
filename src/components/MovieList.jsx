import React, { useState, useEffect, useMemo, useRef } from 'react'
import Fuse from 'fuse.js'
import Header from './Header'
import Footer from './Footer'
import { useKeyboard } from '../hooks/useKeyboard'

const LIST_SIZE = 15 // Number of items to show around center

function MovieList({ onSelect, onBack }) {
  const [movies, setMovies] = useState([])
  const [loading, setLoading] = useState(true)
  const [selectedIndex, setSelectedIndex] = useState(0)
  const [filter, setFilter] = useState('')
  const listRef = useRef(null)

  // Calculate "ends" time - just for visual prototype, we'll use current time + 2h
  const [endsTime, setEndsTime] = useState('')

  useEffect(() => {
    // Update ends time every minute
    const updateEndsTime = () => {
      const now = new Date()
      now.setHours(now.getHours() + 2) // Mock duration
      setEndsTime(`ends ${now.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}`)
    }
    updateEndsTime()
    const timer = setInterval(updateEndsTime, 60000)
    return () => clearInterval(timer)
  }, [])

  useEffect(() => {
    console.log("MovieList mounted, fetching media...")
    window.electronAPI.listMedia()
      .then(files => {
        console.log("Media fetched:", files)
        setMovies(files)
        setLoading(false)
      })
      .catch(err => {
        console.error("Failed to list media:", err)
        setLoading(false)
      })
  }, [])

  const filteredMovies = useMemo(() => {
    if (!filter) return movies
    const fuse = new Fuse(movies, { keys: ['name'], threshold: 0.4 })
    return fuse.search(filter).map(result => result.item)
  }, [movies, filter])

  useEffect(() => {
    setSelectedIndex(0)
  }, [filter])

  useKeyboard({
    ArrowUp: () => setSelectedIndex(prev => Math.max(0, prev - 1)),
    ArrowDown: () => setSelectedIndex(prev => Math.min(filteredMovies.length - 1, prev + 1)),
    Enter: () => {
      if (filteredMovies[selectedIndex]) {
        onSelect(filteredMovies[selectedIndex])
      }
    },
    Backspace: () => {
      if (filter) {
        setFilter(prev => prev.slice(0, -1))
      } else {
        onBack()
      }
    },
    Escape: () => {
      if (filter) {
        setFilter(prev => prev.slice(0, -1))
      } else {
        onBack()
      }
    },
    default: (event) => {
      if (event.key.length === 1) {
        setFilter(prev => prev + event.key)
      }
    }
  })

  // Centering Logic
  // We want the selected item to be vertically centered.
  // Assuming list viewport is 100vh - header/footer adjustments.
  // Each item is ~2.5rem + margins.
  
  if (loading) {
    return (
      <div className="movie-list-container">
        <Header />
        <div style={{ fontSize: '2rem', padding: '50px' }}>LOADING...</div>
      </div>
    )
  }

  // Determine footer left text
  const footerLeft = filter ? (
    <span><span className="search-input">{filter}</span><span className="cursor">_</span></span>
  ) : (
    <span style={{color: '#333'}}>type to search</span>
  )

  if (filteredMovies.length === 0) {
    return (
      <div className="movie-list-container">
        <Header />
        <div style={{ fontSize: '2rem', padding: '50px', marginTop: '100px' }}>
          {filter ? 'NO MATCHES FOUND' : 'NO MOVIES FOUND'}
        </div>
        <Footer leftText={footerLeft} rightText={endsTime} />
      </div>
    )
  }

  return (
    <div className="movie-list-container">
      <Header />
      
      <div className="movie-list-viewport">
        <div 
          className="movie-list" 
          ref={listRef}
          style={{ 
            // 50vh is center. 3rem is roughly item height with margin.
            // Adjustment calculation can be tuned.
            transform: `translateY(calc(50vh - ${(selectedIndex * 3.5) + 1.75}rem))` 
          }}
        >
          {filteredMovies.map((movie, index) => {
            const isSelected = index === selectedIndex;
            // Brutalist fading:
            // Just use opacity steps or color changes if desired.
            // Spec said: "elements near top and bottom gradually turn to dark gray"
            // We already have CSS opacity/color transition setup via classes maybe?
            // Let's use inline styles for the graying out distance.
            const distance = Math.abs(index - selectedIndex);
            let color = '#aaaaaa';
            if (isSelected) color = 'var(--accent-color)';
            else if (distance > 5) color = '#333333';
            else if (distance > 2) color = '#666666';

            return (
              <div
                key={movie.path}
                className={`movie-item ${isSelected ? 'selected' : ''}`}
                style={{ 
                  color: color, 
                  height: '3.5rem', 
                  lineHeight: '3.5rem',
                  // Only render visible items optimization
                  display: distance > 10 ? 'none' : 'block'
                }}
              >
                {isSelected ? `> ${movie.name} <` : `\u00A0\u00A0${movie.name}`}
              </div>
            );
          })}
        </div>
      </div>

      <Footer leftText={footerLeft} rightText={endsTime} />
    </div>
  )
}

export default MovieList