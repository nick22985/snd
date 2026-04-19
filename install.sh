#!/usr/bin/env bash
set -euo pipefail

cargo build --release

cp target/release/snd ~/.local/bin/snd

mkdir -p ~/.config/snd

echo "Installed snd to ~/.local/bin/snd"
echo ""
echo "To enable completions, add this to your shell rc file:"
echo ""
echo "  # Bash (~/.bashrc)"
echo '  source <(COMPLETE=bash snd)'
echo ""
echo "  # Zsh (~/.zshrc)"
echo '  source <(COMPLETE=zsh snd)'
echo ""
echo "  # Fish (~/.config/fish/config.fish)"
echo '  COMPLETE=fish snd | source'
echo ""
echo "Then restart your shell."
