import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { BrowserRouter, MemoryRouter } from 'react-router';
import { isDesktop } from './shared/lib/platform';
import App from './app/App';
import './styles.css';
const Router = isDesktop() ? MemoryRouter : BrowserRouter;
createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <Router>
      <App />
    </Router>
  </StrictMode>,
);
