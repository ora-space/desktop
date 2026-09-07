const cargoPackageVersionPattern =
  /(^\[package\][\s\S]*?^version\s*=\s*")([^"]+)(")/m;

/** Replaces the package version in a Cargo manifest or throws when it is absent. */
export function replaceCargoPackageVersion(
  cargoManifest: string,
  version: string,
) {
  const packageVersionMatch = cargoManifest.match(cargoPackageVersionPattern);
  if (!packageVersionMatch) {
    throw new Error("Could not find the package version in the Cargo manifest");
  }

  return cargoManifest.replace(cargoPackageVersionPattern, `$1${version}$3`);
}
