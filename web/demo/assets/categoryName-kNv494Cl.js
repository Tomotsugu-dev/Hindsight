const d={code:"编程",browse:"浏览",talk:"社交",design:"设计",fun:"娱乐",other:"其他"};function o(e,i){const n=d[e.id];return n!==void 0&&e.name===n?i(`categories.defaults.${e.id}`):e.name}export{o as d};
