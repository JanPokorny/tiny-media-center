import React, { useState } from 'react'
import Header from './Header'
import { useKeyboard } from '../hooks/useKeyboard'

const MENU_ITEMS = ['movies', 'shows', 'wii', 'music', 'youtube']

function MainMenu({ onSelect }) {
  const [selectedIndex, setSelectedIndex] = useState(0)

  useKeyboard({
    ArrowUp: () => setSelectedIndex(prev => Math.max(0, prev - 1)),
    ArrowDown: () => setSelectedIndex(prev => Math.min(MENU_ITEMS.length - 1, prev + 1)),
    Enter: () => onSelect(MENU_ITEMS[selectedIndex])
  })

  return (
    <div className="main-menu">
      <Header />
      <div className="menu-list">
        {MENU_ITEMS.map((item, index) => (
          <div
            key={item}
            className={`menu-item ${index === selectedIndex ? 'selected' : ''}`}
          >
            {index === selectedIndex ? `> ${item} <` : `\u00A0\u00A0${item}`}
          </div>
        ))}
      </div>
    </div>
  )
}

export default MainMenu