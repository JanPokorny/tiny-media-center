import React, { useState, useEffect, useMemo } from 'react'
import Fuse from 'fuse.js'
import Header from './Header'
import Footer from './Footer'
import { useKeyboard } from '../hooks/useKeyboard'

function MediaBrowser({ items, onSelect, onBack, currentPath }) {
  const [selectedIndex, setSelectedIndex] = useState(0)
  const [filter, setFilter] = useState('')

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
    Backspace: () => {
      if (filter) {
        setFilter(prev => prev.slice(0, -1))
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
      <Header title={currentFolderName} />
      
      <div className="movie-list-viewport">
        <div 
          className="movie-list"
          style={{ 
            transform: `translateY(calc(50vh - ${(selectedIndex * 3.5) + 1.75}rem))` 
          }}
        >
          {filteredItems.map((item, index) => {
            const isSelected = index === selectedIndex;
            const distance = Math.abs(index - selectedIndex);
            let color = '#aaaaaa';
            if (isSelected) color = 'var(--accent-color)';
            else if (distance > 5) color = '#333333';
            else if (distance > 2) color = '#666666';

            const displayName = item.type === 'directory' ? `${item.name}/` : item.name;

            return (
              <div
                key={item.path}
                className={`movie-item ${isSelected ? 'selected' : ''}`}
                style={{ 
                  color: color, 
                  height: '3.5rem',
                  lineHeight: '3.5rem',
                  display: distance > 10 ? 'none' : 'block'
                }}
              >
                {isSelected ? `> ${displayName}` : `\u00A0\u00A0${displayName}`}
              </div>
            );
          })}
        </div>
      </div>

      <Footer leftText={footerLeft} />
    </div>
  )
}

export default MediaBrowser
