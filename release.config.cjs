const { releasePlugins } = require("./tools/semantic-release-plugins.cjs");

// Forgejo owns branches. GitHub owns tag-only releases of mirrored commits.
// The future GitHub repository is supplied by Actions, never guessed here.
module.exports = {
  branches: ["main"],
  tagFormat: "v${version}",
  plugins: releasePlugins,
};
