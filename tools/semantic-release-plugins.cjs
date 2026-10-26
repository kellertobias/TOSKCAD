const path = require("node:path");

const analyze = [
  require.resolve("@semantic-release/commit-analyzer"),
  {
    preset: "conventionalcommits",
    releaseRules: [
      { breaking: true, release: "major" },
      { type: "fix", release: "patch" },
      { type: "perf", release: "patch" },
      ...["docs", "test", "style", "refactor", "build", "ci", "chore"].map(
        (type) => ({ type, release: false }),
      ),
    ],
  },
];
const notes = [
  require.resolve("@semantic-release/release-notes-generator"),
  { preset: "conventionalcommits" },
];
const versionOutput = path.join(__dirname, "semantic-release-version-output.cjs");

module.exports = {
  previewPlugins: [analyze, notes, versionOutput],
  releasePlugins: [
    analyze,
    notes,
    versionOutput,
    [
      require.resolve("@semantic-release/github"),
      {
        assets: [
          "artifacts/release/*.zip",
          "artifacts/release/*.sha256",
          "artifacts/release/release-manifest.json",
        ],
        successComment: false,
        failComment: false,
        releasedLabels: false,
        addReleases: false,
      },
    ],
  ],
};
