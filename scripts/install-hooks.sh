#!/bin/sh
# Install git pre-commit hook for construct-tui.
# Run once after cloning: sh scripts/install-hooks.sh

set -e

HOOK=.git/hooks/pre-commit

cat > "$HOOK" << 'EOF'
#!/bin/sh
# Pre-commit: cargo fmt check + clippy
set -e

if ! command -v cargo >/dev/null 2>&1; then
    echo "cargo not found, skipping pre-commit checks"
    exit 0
fi

echo "[pre-commit] cargo fmt --check..."
if ! cargo fmt --all -- --check 2>&1; then
    echo ""
    echo "Formatting issues found. Auto-fixing with cargo fmt --all..."
    cargo fmt --all
    echo ""
    echo "Re-stage the reformatted files and commit again:"
    git diff --name-only
    exit 1
fi
echo "[pre-commit] fmt: OK"

echo "[pre-commit] cargo clippy..."
cargo clippy --all-targets -- -D warnings 2>&1
echo "[pre-commit] clippy: OK"
EOF

chmod +x "$HOOK"
echo "Pre-commit hook installed at $HOOK"
