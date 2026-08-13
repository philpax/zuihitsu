import { useEffect, useState } from "react";

import type { PackageBlob } from "@zuihitsu/wire/types/PackageBlob.ts";
import { servedMediaType, whenWasmReady } from "../replica/replica.ts";
import { OCTET_STREAM } from "../api/blobs.ts";
import { NO_BLOBS, type BlobSource } from "./blobSource.ts";

/// The eval frame's [`BlobSource`]: one object URL per catalogue entry, revoked when the catalogue
/// changes or the frame unmounts. An address the catalogue does not carry resolves to `null`.
///
/// Object URLs rather than `data:` URIs because a browser blocks top-level navigation to `data:`,
/// which would leave the open-the-file link and the download chip doing nothing. A `blob:` URL
/// navigates, downloads, and renders in an `<img>` as a served one does.
///
/// The catalogue is optional (a package may carry none) and falls back to a shared constant, since a
/// fresh `[]` per render would re-mint every URL.
export function usePackageBlobs(blobs: readonly PackageBlob[] | undefined): BlobSource {
  const catalogue = blobs ?? NO_CATALOGUE;
  // Keyed by the catalogue it was minted from and matched during render, as `useReplica` and
  // `useRunRecord` are: a change reads as unresolved without a state write in the effect.
  const [minted, setMinted] = useState<{
    catalogue: readonly PackageBlob[];
    urls: ReadonlyMap<string, string>;
  } | null>(null);

  useEffect(() => {
    if (catalogue.length === 0) return;
    let revoked = false;
    let urls: string[] = [];
    // The media type comes from Rust, and this frame renders before any `Replica` loads the module.
    void whenWasmReady().then(() => {
      if (revoked) return;
      const map = new Map<string, string>();
      for (const blob of catalogue) {
        const url = URL.createObjectURL(objectBlob(blob));
        urls.push(url);
        map.set(blob.hash, url);
      }
      setMinted({ catalogue, urls: map });
    });
    return () => {
      revoked = true;
      for (const url of urls) URL.revokeObjectURL(url);
      urls = [];
    };
  }, [catalogue]);

  if (minted === null || minted.catalogue !== catalogue) return NO_BLOBS;
  return { urlFor: ({ blob }) => minted.urls.get(blob) ?? null };
}

const NO_CATALOGUE: readonly PackageBlob[] = [];

/// One catalogue entry as a `Blob`, typed as the agent's read route would serve it rather than as it
/// was stored: markup a run shared reads as text here too.
///
/// An object URL carries no headers, so it cannot state `nosniff` the way the read route does. A type
/// a browser treats as unknown is sniffed instead — and a package is a file from anywhere — so an
/// unrecognised type floors to the generic one, which is sniffed as bytes rather than as markup.
export function objectBlob(blob: PackageBlob): Blob {
  const served = servedMediaType(blob.mime);
  return new Blob([decodeBase64(blob.base64)], {
    type: SNIFFABLE.has(served.trim().toLowerCase()) ? OCTET_STREAM : served,
  });
}

/// The types the MIME Sniffing standard (§7) calls unknown, and so sniffs the body for.
const SNIFFABLE = new Set(["", "*/*", "unknown/unknown", "application/unknown"]);

/// Decode standard-alphabet base64. `atob` yields one character per byte, so each code unit is the
/// byte; the buffer is explicit because a `Blob` part needs an `ArrayBuffer`, not an `ArrayBufferLike`.
export function decodeBase64(base64: string): Uint8Array<ArrayBuffer> {
  const binary = atob(base64);
  const bytes = new Uint8Array(new ArrayBuffer(binary.length));
  for (let index = 0; index < binary.length; index += 1) {
    bytes[index] = binary.charCodeAt(index);
  }
  return bytes;
}
