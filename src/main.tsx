import React from "react";
import ReactDOM from "react-dom/client";
import { BrowserRouter } from "react-router-dom";
import App from "./App";
import { DeviceFilterProvider } from "./state/deviceFilter";
import { CategoriesProvider } from "./state/categories";
import { SettingsProvider } from "./state/settings";
import "./styles/global.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <BrowserRouter>
      <SettingsProvider>
        <CategoriesProvider>
          <DeviceFilterProvider>
            <App />
          </DeviceFilterProvider>
        </CategoriesProvider>
      </SettingsProvider>
    </BrowserRouter>
  </React.StrictMode>,
);
