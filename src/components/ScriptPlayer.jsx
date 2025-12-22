import { useState, useEffect } from 'react';
import Header from './Header';
import Footer from './Footer';
import { useKeyboard } from '../hooks/useKeyboard';

function ScriptPlayer({ script, onBack }) {
  const [output, setOutput] = useState('');

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
      <Header title={`running ${script.name}`} />
      <pre className="script-output">
        {output}
      </pre>
      <Footer leftText="press 'back' to exit" />
    </>
  );
}

export default ScriptPlayer;
