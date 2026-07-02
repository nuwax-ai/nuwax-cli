#!/usr/bin/env node
/**
 * Patch cargo-dist npm package to download binaries from Alibaba Cloud OSS
 * instead of GitHub Releases.
 *
 * Usage:
 *   node scripts/patch-npm-oss-package.mjs --channel beta|latest --tag-name <tag> <npm-package.tar.gz>
 */

import { readFileSync, writeFileSync, mkdirSync, rmSync } from "fs";
import { execSync } from "child_process";
import { join } from "path";

const OSS_BASE =
  "https://nuwa-packages.oss-rg-china-mainland.aliyuncs.com";

const OSS_PLATFORM_ARTIFACTS = {
  "aarch64-apple-darwin": {
    artifactName: "nuwax-cli-macos-arm64.tar.gz",
    zipExt: ".tar.gz",
  },
  "x86_64-apple-darwin": {
    artifactName: "nuwax-cli-macos-universal.tar.gz",
    zipExt: ".tar.gz",
  },
  "aarch64-unknown-linux-gnu": {
    artifactName: "nuwax-cli-linux-arm64.tar.gz",
    zipExt: ".tar.gz",
  },
  "x86_64-unknown-linux-gnu": {
    artifactName: "nuwax-cli-linux-amd64.tar.gz",
    zipExt: ".tar.gz",
  },
  "x86_64-pc-windows-msvc": {
    artifactName: "nuwax-cli-windows-amd64.zip",
    zipExt: ".zip",
  },
  "x86_64-pc-windows-gnu": {
    artifactName: "nuwax-cli-windows-amd64.zip",
    zipExt: ".zip",
  },
  "aarch64-pc-windows-msvc": {
    artifactName: "nuwax-cli-windows-arm64.zip",
    zipExt: ".zip",
  },
};

function parseArgs() {
  const args = process.argv.slice(2);
  let channel;
  let tagName;
  let pkgPath;

  for (let i = 0; i < args.length; i++) {
    if (args[i] === "--channel") {
      channel = args[++i];
    } else if (args[i] === "--tag-name") {
      tagName = args[++i];
    } else if (!pkgPath) {
      pkgPath = args[i];
    }
  }

  if (!channel || !tagName || !pkgPath) {
    console.error(
      "Usage: patch-npm-oss-package.mjs --channel beta|latest --tag-name <tag> <npm-package.tar.gz>",
    );
    process.exit(1);
  }

  if (channel !== "beta" && channel !== "latest") {
    console.error(`Unsupported channel: ${channel}`);
    process.exit(1);
  }

  return { channel, tagName, pkgPath };
}

function ossDownloadBase(channel, tagName) {
  if (channel === "beta") {
    return `${OSS_BASE}/nuwax-cli/beta/${tagName}`;
  }
  return `${OSS_BASE}/nuwax-cli/${tagName}`;
}

function patchPackage(pkgPath, channel, tagName) {
  const workDir = join("/tmp", `npm-oss-patch-${Date.now()}`);
  mkdirSync(workDir, { recursive: true });

  try {
    execSync(`tar -xzf "${pkgPath}" -C "${workDir}"`, { stdio: "inherit" });

    const pkgJsonPath = join(workDir, "package", "package.json");
    const pkg = JSON.parse(readFileSync(pkgJsonPath, "utf8"));
    const downloadUrl = ossDownloadBase(channel, tagName);

    pkg.artifactDownloadUrls = [downloadUrl];

    for (const [triple, oss] of Object.entries(OSS_PLATFORM_ARTIFACTS)) {
      if (pkg.supportedPlatforms?.[triple]) {
        pkg.supportedPlatforms[triple].artifactName = oss.artifactName;
        pkg.supportedPlatforms[triple].zipExt = oss.zipExt;
      }
    }

    writeFileSync(pkgJsonPath, `${JSON.stringify(pkg, null, 2)}\n`);

    const outPath = `${pkgPath}.oss.tmp`;
    execSync(`tar -czf "${outPath}" -C "${workDir}" package`, { stdio: "inherit" });
    execSync(`mv "${outPath}" "${pkgPath}"`, { stdio: "inherit" });

    console.log(`✅ Patched npm package for OSS downloads`);
    console.log(`   channel: ${channel}`);
    console.log(`   base URL: ${downloadUrl}`);
    console.log(`   package: ${pkgPath}`);
  } finally {
    rmSync(workDir, { recursive: true, force: true });
  }
}

const { channel, tagName, pkgPath } = parseArgs();
patchPackage(pkgPath, channel, tagName);
