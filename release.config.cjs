const { releasePlugins } = require("./tools/semantic-release-plugins.cjs");

// GitHub owns branches and tag-only releases on tested commits.
// Actions supplies the repository; no tracked release/version commits.
module.exports = {
  branches: ["main"],
  tagFormat: "v${version}",
  plugins: releasePlugins,
};
