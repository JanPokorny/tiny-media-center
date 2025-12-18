import { useEffect } from 'react'

export function useKeyboard(keyMap) {
  useEffect(() => {
    const handleKeyDown = (event) => {
      const handler = keyMap[event.key] || keyMap['default']
      if (handler) {
        handler(event)
      }
    }

    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [keyMap])
}
