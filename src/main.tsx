import React from "react";
import ReactDOM from "react-dom/client";
import { BrowserRouter } from "react-router-dom";
import App from "./App";
import { DeviceFilterProvider } from "./state/deviceFilter";
import { CategoriesProvider } from "./state/categories";
import "./styles/global.css";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <BrowserRouter>
      <CategoriesProvider>
        <DeviceFilterProvider>
          <App />
        </DeviceFilterProvider>
      </CategoriesProvider>
    </BrowserRouter>
  </React.StrictMode>,
);
