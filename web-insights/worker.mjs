import html from "./dashboard.html";
import css from "./dashboard.css";
import javascript from "./dashboard.js";
import downloadsModule from "../scripts/insights/downloads.mjs";
import config from "../scripts/insights/config.json";
import { createHandler } from "./handler.mjs";

export default createHandler({ html, css, javascript, downloadsModule, repositories: config.repositories });
