import { useState, useEffect } from 'react';
import { useKeyboard } from '../hooks/useKeyboard';

function ScriptRunner({ script, onBack }) {
  const [output, setOutput] = useState('');
  const [time, setTime] = useState(new Date());

  useEffect(() => {
    const timer = setInterval(() => setTime(new Date()), 1000)
    return () => clearInterval(timer)
  }, [])

  useEffect(() => {
    setOutput('');

    const removeOutputListener = window.ipcRenderer.on(
      'script-output',
      (_event, value) => setOutput(prev => prev + value)
    );
    window.ipcRenderer.send('run-script', script.path);

    return () => {
      removeOutputListener();
      window.ipcRenderer.send('kill-script');
    };
  }, [script]);

  useKeyboard({
    Backspace: onBack,
    Escape: onBack,
  });

  return (
    <>
      <header>
        <span>{`running ${script.name}`}</span>
        <span>
          {time.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', hour12: false })}
        </span>
      </header>
      <pre className="script-output">
        {output}
      </pre>
      <footer>
        <span>press 'back' to exit</span>
        <span></span>
      </footer>
    </>
  );
}

export default ScriptRunner;
