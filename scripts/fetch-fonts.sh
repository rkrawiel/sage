#!/usr/bin/env sh
# Downloads the bundled fonts (all SIL OFL 1.1) into assets/fonts.
# The fonts are committed with the project, so building does not need this. Use it
# to add a font (delete a file first to download it again), and add the font's
# licence to assets/fonts/licenses.
set -eu
cd "$(dirname "$0")/../assets/fonts"
get() { [ -f "$1" ] || curl -fsSL -o "$1" "$2"; }
get SourceSerif4-Regular.ttf  https://raw.githubusercontent.com/adobe-fonts/source-serif/release/TTF/SourceSerif4-Regular.ttf
get SourceSerif4-Semibold.ttf https://raw.githubusercontent.com/adobe-fonts/source-serif/release/TTF/SourceSerif4-Semibold.ttf
get SourceSans3-Regular.ttf   https://raw.githubusercontent.com/adobe-fonts/source-sans/release/TTF/SourceSans3-Regular.ttf
get SourceSans3-Semibold.ttf  https://raw.githubusercontent.com/adobe-fonts/source-sans/release/TTF/SourceSans3-Semibold.ttf
get SourceSans3-It.ttf        https://raw.githubusercontent.com/adobe-fonts/source-sans/release/TTF/SourceSans3-It.ttf
get SourceSans3-SemiboldIt.ttf https://raw.githubusercontent.com/adobe-fonts/source-sans/release/TTF/SourceSans3-SemiboldIt.ttf
get JetBrainsMono-Regular.ttf https://raw.githubusercontent.com/JetBrains/JetBrainsMono/master/fonts/ttf/JetBrainsMono-Regular.ttf
get NotoEmoji.ttf             "https://raw.githubusercontent.com/google/fonts/main/ofl/notoemoji/NotoEmoji%5Bwght%5D.ttf"
echo "fonts ready in $(pwd)"
