import html from "./dashboard.html";
import css from "./dashboard.css";
import javascript from "./dashboard.js";
import config from "../scripts/insights/config.json";
import { createHandler } from "./handler.mjs";

export default createHandler({ html, css, javascript, repositories: config.repositories });
