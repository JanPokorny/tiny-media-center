import { useState, useEffect, useMemo } from 'react'
import Fuse from 'fuse.js'
import { useKeyboard } from '../hooks/useKeyboard'

function MediaBrowser({ items, onSelect, onBack, currentPath }) {
  const [selectedIndex, setSelectedIndex] = useState(0)
  const [filter, setFilter] = useState('')
  const [time, setTime] = useState(new Date())

  useEffect(() => {
    const timer = setInterval(() => setTime(new Date()), 1000)
    return () => clearInterval(timer)
  }, [])

  const filteredItems = useMemo(() => {
    if (!filter) return items
    const fuse = new Fuse(items, { keys: ['name'], threshold: 0.4 })
    return fuse.search(filter).map(result => result.item)
  }, [items, filter])

  useEffect(() => {
    setSelectedIndex(0)
    setFilter('')
  }, [items])

  useKeyboard({
    ArrowUp: () => setSelectedIndex(prev => Math.max(0, prev - 1)),
    ArrowDown: () => setSelectedIndex(prev => Math.min(filteredItems.length - 1, prev + 1)),
    Enter: () => {
      if (filteredItems[selectedIndex]) {
        onSelect(filteredItems[selectedIndex])
      }
    },
    BrowserBack: () => {
      if (filter) {
        setFilter('')
      } else {
        onBack()
      }
    },
    Escape: () => {
      if (filter) {
        setFilter('')
      } else {
        onBack()
      }
    },
    Backspace: () => {
      if (filter) setFilter(prev => prev.slice(0, -1))
    },
    default: (event) => {
      if (event.key.length === 1) {
        setFilter(prev => prev + event.key)
      }
    }
  })

  const footerLeft = filter ? (
    <span><span className="search-input">{filter}</span><span className="cursor">_</span></span>
  ) : (
    <span style={{color: '#333'}}>type to search</span>
  )
  
  const currentFolderName = currentPath ? currentPath.split( /\/|\\/ ).pop() : 'tiny media center';

  return (
    <div className="movie-list-container">
      <header>
        <span>{currentFolderName}</span>
        <span>
          {time.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', hour12: false })}
        </span>
      </header>
      
      <div className="movie-list-viewport">
        <div 
          className="movie-list"
          style={{ 
            transform: `translateY(calc(50vh - ${(selectedIndex * 24) + 27}px))` 
          }}
        >
          {filteredItems.map((item, index) => {
            const isSelected = index === selectedIndex;

            return (
              <div
                key={item.path}
                className={`movie-item ${isSelected ? 'selected' : ''}`}
              >
                {isSelected ? `> ${item.name}` : `\u00A0\u00A0${item.name}`}
              </div>
            );
          })}
        </div>
      </div>

      <footer>
        <span>{footerLeft}</span>
        <span></span>
      </footer>
    </div>
  )
}

export default MediaBrowser
