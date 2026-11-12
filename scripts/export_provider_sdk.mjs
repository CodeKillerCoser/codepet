import { mkdir, readdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { loadProviderContract, publishProviderContract } from '../tools/protocol-codegen/provider-contract.mjs';
const repositoryRoot=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
export async function exportProviderSdk(output, sourceRoot=repositoryRoot) {
  output=path.resolve(output);
  await mkdir(output,{recursive:true});
  if((await readdir(output)).some(name=>name!=='.gitkeep'))throw new Error('SDK export requires an empty output directory; do not mix contract versions or private resources');
  const model=await loadProviderContract(path.join(sourceRoot,'protocol'));
  const versionRoot=path.join(output,`v${model.index.sdkVersion}`);
  for(const [name,content]of publishProviderContract(model)){
    const file=path.join(versionRoot,name);await mkdir(path.dirname(file),{recursive:true});await writeFile(file,content);
  }
  return output;
}
if(process.argv[1]&&pathToFileURL(path.resolve(process.argv[1])).href===import.meta.url){
  if(!process.argv[2])throw new Error('Usage: node scripts/export_provider_sdk.mjs <empty-output-directory>');
  console.log(await exportProviderSdk(process.argv[2]));
}
