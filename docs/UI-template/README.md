# Tasker Web UI design reference

These files are the original design reference supplied for Tasker's optional local Web UI.

- `Cairn.html` is the complete self-contained prototype.
- `Confluence-Jira hybrid design discussion.zip` is the supplied split export.
- `split/` contains the extracted split export for inspection.

The production UI does **not** serve these artifacts directly. They contain a proprietary component runtime, hard-coded example data, and external development dependencies. Runtime assets under `src/ui/assets/` rebuild the information architecture and visual language as offline semantic HTML, CSS, and vanilla JavaScript connected to Tasker's typed local API.

Reference SHA-256 hashes:

```text
42c96a99f452aaa3cf0cdd6a26393322466d9b7ec1a1c99fafe7437a522d0b2a  Cairn.html
d17ec327d5036e6b732a3341b4755e60255d003b017fac0baf3c512580a06877  Confluence-Jira hybrid design discussion.zip
d464ec4ca91c079a87d26724c4ecfbe6b3acbd1effed054cc06ae82b3b53ae4a  split/Cairn.dc.html
8fe7df74405f3c55f49b7249c74ea1397e65d07dea2b1bd3b4a489bec2e28cbe  split/support.js
```
