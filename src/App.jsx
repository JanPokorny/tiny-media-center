import React, { useState } from 'react'
import MainMenu from './components/MainMenu'
import MovieList from './components/MovieList'
import Player from './components/Player'

function App() {
  const [view, setView] = useState('MAIN_MENU')
  const [selectedMovie, setSelectedMovie] = useState(null)

  const renderView = () => {
    switch (view) {
      case 'MAIN_MENU':
        return (
          <MainMenu
            onSelect={(item) => {
              if (item === 'movies') setView('MOVIE_LIST')
            }}
          />
        )
      case 'MOVIE_LIST':
        return (
          <MovieList
            onSelect={(movie) => {
              setSelectedMovie(movie)
              setView('PLAYER')
            }}
            onBack={() => setView('MAIN_MENU')}
          />
        )
      case 'PLAYER':
        return (
          <Player
            movie={selectedMovie}
            onBack={() => {
              setView('MOVIE_LIST')
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