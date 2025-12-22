import React, { useState, useEffect } from 'react'
import path from 'path-browserify'
import MediaBrowser from './components/MediaBrowser'
import Player from './components/Player'

function App() {
  const [mediaTree, setMediaTree] = useState([])
  const [currentPath, setCurrentPath] = useState('')
  const [view, setView] = useState('BROWSER')
  const [selectedMovie, setSelectedMovie] = useState(null)
  const [loading, setLoading] = useState(true)

  useEffect(() => {
    async function fetchMedia() {
      const structure = await window.electronAPI.getMediaStructure()
      setMediaTree(structure)
      setLoading(false)
    }
    fetchMedia()
  }, [])

  const getCurrentItems = () => {
    if (!currentPath) return mediaTree
    
    const parts = currentPath.split(path.sep)
    let items = mediaTree
    for (const part of parts) {
      const dir = items.find(item => item.name === part && item.type === 'directory')
      if (dir) {
        items = dir.children
      } else {
        return []
      }
    }
    return items
  }

  const handleSelect = (item) => {
    if (item.type === 'directory') {
      setCurrentPath(item.path)
    } else {
      setSelectedMovie(item)
      setView('PLAYER')
    }
  }

  const handleBack = () => {
    if (!currentPath) return
    
    const parentPath = path.dirname(currentPath)
    if (parentPath === '.') {
      setCurrentPath('')
    } else {
      setCurrentPath(parentPath)
    }
  }

  const renderView = () => {
    if (loading) {
        return <div>Loading...</div>
    }

    switch (view) {
      case 'BROWSER':
        return (
          <MediaBrowser
            items={getCurrentItems()}
            currentPath={currentPath}
            onSelect={handleSelect}
            onBack={handleBack}
          />
        )
      case 'PLAYER':
        return (
          <Player
            movie={selectedMovie}
            onBack={() => {
              setView('BROWSER')
              setSelectedMovie(null)
            }}
          />
        )
      default:
        return <div>Unknown View</div>
    }
  }

  return <div className="app">{renderView()}</div>
}

export default App