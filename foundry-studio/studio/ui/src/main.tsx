// Where the page starts (spec 02 section 3.1, AppShell): the stored theme applied before the first paint, then one
// React root in strict mode drawing the Studio shell under React Router 7's declarative router (brief I1.05, lead
// decided). Every address reaches the shell, whose own route table (spec 03 section 3.2) reads it.

import { StrictMode } from 'react';
import { createRoot } from 'react-dom/client';
import { BrowserRouter, Route, Routes } from 'react-router';
import { applyTheme, preferences } from './shell/preferences.ts';
import { Studio } from './shell/Studio.tsx';
import './styles/tokens.css';
import './styles/studio.css';

applyTheme(preferences().theme);

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <BrowserRouter>
      <Routes>
        <Route path="*" element={<Studio />} />
      </Routes>
    </BrowserRouter>
  </StrictMode>,
);
