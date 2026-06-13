import type { EvalPackage } from "../types/EvalPackage.ts";

/// Parse an eval package from a dropped or chosen file. The package is a plain JSON document whose
/// shape is the ts-rs `EvalPackage` contract, so no validation beyond the parse is needed here.
export async function loadPackageFromFile(file: File): Promise<EvalPackage> {
  const text = await file.text();
  return JSON.parse(text) as EvalPackage;
}
