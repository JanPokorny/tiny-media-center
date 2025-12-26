import { useState, useEffect, useMemo } from 'react'
import Fuse from 'fuse.js'
import { useKeyboard } from '../hooks/useKeyboard'
import Menu from './Menu'

function MediaBrowser({ items, onSelect, onBack, currentPath, initialSelectedIndex }) {
  const [selectedIndex, setSelectedIndex] = useState(initialSelectedIndex || 0)
  const [filter, setFilter] = useState('')

  const filteredItems = useMemo(() => {
    if (!filter) return items
    const fuse = new Fuse(items, { keys: ['name'], threshold: 0.4 })
    return fuse.search(filter).map(result => result.item)
  }, [items, filter])

  useEffect(() => {
    setSelectedIndex(initialSelectedIndex || 0)
    setFilter('')
  }, [items, initialSelectedIndex])

  const handleBack = () => {
    if (filter) {
      setFilter('')
    } else {
      onBack()
    }
  }

  useKeyboard({
    ArrowUp: () => setSelectedIndex(prev => Math.max(0, prev - 1)),
    ArrowDown: () => setSelectedIndex(prev => Math.min(filteredItems.length - 1, prev + 1)),
    Enter: () => {
      if (filteredItems[selectedIndex]) {
        onSelect(filteredItems[selectedIndex], selectedIndex)
      }
    },
    Escape: handleBack,
    BrowserBack: handleBack,
    Backspace: () => {
      if (filter) setFilter(prev => prev.slice(0, -1))
    },
    default: (event) => {
      if (event.key.length === 1) {
        setFilter(prev => prev + event.key)
      }
    }
  }, [filter, filteredItems, selectedIndex])


  const footerLeft = filter ? (
    <span><span className="search-input">{filter}</span><span className="cursor">_</span></span>
  ) : (
    <span style={{color: '#333'}}>type to search</span>
  )
  
  const currentFolderName = currentPath ? currentPath.split( /\/|\\/ ).pop() : 'tiny media center';

  return (
    <div className="movie-list-container">
      <Menu
        items={filteredItems}
        title={currentFolderName}
        selectedIndex={selectedIndex}
      />
      <footer>
        <span>{footerLeft}</span>
        <span></span>
      </footer>
    </div>
  )
}

export default MediaBrowser
