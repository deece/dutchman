# Desktop walkthrough images

`readme-drop-day.png`, `readme-preset-day.png`, `readme-result-day.png`, and
`readme-result-night.png` show the real desktop UI rendered by the ignored
`readme_screenshots` GPU test in `crates/dutchman-gui/tests/ui.rs`.

The source file is `crates/dutchman-core/tests/fixtures/design_history.FCStd`,
shown as `Bracket.FCStd`. It is cadmpeg's CC0-1.0 synthetic FreeCAD history fixture;
its origin and checksum are recorded in the fixtures README. No private designs
are used. HOME and XDG folders are temporary, and a local alias keeps the real
SolidCraft installation path out of the images. The conversion itself uses the
real SolidCraft CLI to check history. PNGs use the image crate's best compression.

To regenerate on Linux with a GPU, set `SOLIDCRAFT_CLI` to a released CLI and
`DUTCHMAN_GUI_SCREENSHOTS` to an absolute output folder, then run the ignored
`readme_screenshots` test with the `test-support` feature.
