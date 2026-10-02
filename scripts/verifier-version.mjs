import { readFileSync } from 'node:fs';

const paquet = JSON.parse(readFileSync(new URL('../package.json', import.meta.url)));
const verrou = JSON.parse(readFileSync(new URL('../package-lock.json', import.meta.url)));
const tag = process.env.GITHUB_REF_TYPE === 'tag' ? process.env.GITHUB_REF_NAME : process.argv[2];
if (paquet.version !== verrou.version || paquet.version !== verrou.packages[''].version) {
  throw new Error('Les versions package.json et package-lock.json divergent.');
}
if (tag && tag !== `v${paquet.version}`) {
  throw new Error(`Tag ${tag} incompatible avec la version ${paquet.version}.`);
}
console.log(`Version ${paquet.version} coherente${tag ? ` avec ${tag}` : ''}.`);
