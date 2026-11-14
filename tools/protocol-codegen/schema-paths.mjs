import path from 'node:path';

// Schema refs are local URI paths; keep filesystem keys native on every platform.
export function resolveSchemaPath(sourcePath, reference, paths = path) {
  return reference ? paths.resolve(paths.dirname(sourcePath), decodeURIComponent(reference)) : sourcePath;
}

export function legacySchemaPath(schemaPath, sourceVersion, paths = path) {
  const directory = paths.dirname(schemaPath);
  if (sourceVersion === undefined ? !/^v[1-9][0-9]*$/.test(paths.basename(directory)) : paths.basename(directory) !== `v${sourceVersion}`) {
    throw new Error(`Unexpected versioned schema path: ${schemaPath}`);
  }
  return paths.join(paths.dirname(directory), 'v1', paths.basename(schemaPath));
}
