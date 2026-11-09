// Puts all of Hindsight's own CSS in the `app` cascade layer. A theme's theme.css is added at run
// time and is not in a layer, so its rules always win over Hindsight's, whatever their specificity.
//
// Vite finds this file from both the app and the web demo, so theme previews on the website
// behave like the app. CSS modules are compiled before this plugin runs. Do not use `composes`
// from another file: the CSS it pulls in skips this plugin and ends up outside the layer.

/** @type {import("postcss").Plugin} */
const appLayer = {
  postcssPlugin: "hindsight-app-layer",
  Once(root, { AtRule }) {
    // Without `nodes`, an empty file would leave layer.nodes undefined
    const layer = new AtRule({ name: "layer", params: "app", nodes: [] });
    for (const node of [...root.nodes]) {
      // @charset and @import must stay at the top level
      if (node.type === "atrule" && (node.name === "charset" || node.name === "import")) continue;
      // Appending moves the node out of the root
      layer.append(node);
    }
    if (layer.nodes.length > 0) root.append(layer);
  },
};

export default {
  plugins: [appLayer],
};
