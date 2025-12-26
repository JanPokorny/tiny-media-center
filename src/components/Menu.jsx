import { useState, useEffect } from 'react'

function Menu({ items, onSelect, onBack, title, selectedIndex }) {
  const [time, setTime] = useState(new Date())

  useEffect(() => {
    const timer = setInterval(() => setTime(new Date()), 1000)
    return () => clearInterval(timer)
  }, [])

  return (
    <>
      <header>
        <span>{title}</span>
        <span>
          {time.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', hour12: false })}
        </span>
      </header>
      
      <div className="movie-list-viewport">
        <div 
          className="movie-list"
          style={{ 
            transform: `translateY(calc(50vh - ${(selectedIndex * 54) + 27}px))` 
          }}
        >
          {items.map((item, index) => {
            const isSelected = index === selectedIndex;

            return (
              <div
                key={item.key || item.path}
                className={`movie-item ${isSelected ? 'selected' : ''}`}
              >
                {isSelected ? `> ${item.label || item.name}` : `\u00A0\u00A0${item.label || item.name}`}
              </div>
            );
          })}
        </div>
      </div>
    </>
  )
}

export default Menu;
