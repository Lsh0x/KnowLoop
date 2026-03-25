# typed: false
# frozen_string_literal: true

# Homebrew formula for KnowLoop
#
# Install: brew install Lsh0x/tap/knowloop
#
# This formula downloads pre-built binaries from GitHub Releases.
# SHA256 checksums are updated automatically by the release CI.
class KnowLoop < Formula
  desc "AI agent orchestrator with Neo4j knowledge graph, Meilisearch, and Tree-sitter"
  homepage "https://github.com/Lsh0x/KnowLoop"
  license "MIT"
  version "VERSION_PLACEHOLDER"

  on_macos do
    on_arm do
      url "https://github.com/Lsh0x/KnowLoop/releases/download/v#{version}/knowloop-full-#{version}-macos-arm64.tar.gz"
      sha256 "SHA256_MACOS_ARM64_PLACEHOLDER"
    end

    on_intel do
      url "https://github.com/Lsh0x/KnowLoop/releases/download/v#{version}/knowloop-full-#{version}-macos-x86_64.tar.gz"
      sha256 "SHA256_MACOS_X86_64_PLACEHOLDER"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Lsh0x/KnowLoop/releases/download/v#{version}/knowloop-full-#{version}-linux-arm64.tar.gz"
      sha256 "SHA256_LINUX_ARM64_PLACEHOLDER"
    end

    on_intel do
      url "https://github.com/Lsh0x/KnowLoop/releases/download/v#{version}/knowloop-full-#{version}-linux-x86_64.tar.gz"
      sha256 "SHA256_LINUX_X86_64_PLACEHOLDER"
    end
  end

  def install
    bin.install "knowloop"
    bin.install "kl"
    bin.install "knowloop_mcp"

    # ONNX Runtime dylib — present only in macOS x86_64 builds (dynamic linking
    # because ort-sys has no prebuilt static library for macOS Intel).
    # Binaries have @executable_path/../lib in their rpath for this layout.
    lib.install Dir["libonnxruntime*"] unless Dir["libonnxruntime*"].empty?
  end

  def caveats
    <<~EOS
      To start the server:
        brew services start knowloop
        # or: knowloop serve

      To configure Claude Code integration:
        knowloop setup-claude
        (auto-configures the MCP server in Claude Code)

      The MCP server binary is at: #{opt_bin}/knowloop_mcp
      The CLI tool is at: #{opt_bin}/kl

      Before starting, ensure Neo4j and MeiliSearch are running.
    EOS
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/knowloop --version")
  end

  service do
    run [opt_bin/"knowloop", "serve"]
    keep_alive true
    working_dir var/"knowloop"
    log_path var/"log/knowloop.log"
    error_log_path var/"log/knowloop-error.log"
  end
end
