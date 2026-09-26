import { readFile, readdir, stat } from 'node:fs/promises';
import { dirname, resolve, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const errors = [];
const english = (await readdir(resolve(root, 'docs/en'))).filter(name => name.endsWith('.md')).sort();
const spanish = (await readdir(resolve(root, 'docs/es'))).filter(name => name.endsWith('.md')).sort();
if (JSON.stringify(english) !== JSON.stringify(spanish)) errors.push('EN/ES document inventories differ.');
const pairs = [['README.md', 'README.es.md'], ...english.map(name => [`docs/en/${name}`, `docs/es/${name}`])];
const files = [...pairs.flat(), 'THIRD_PARTY_NOTICES.md'];
const stripCode = text => text.replace(/^```[^\n]*\n[\s\S]*?^```\s*$/gm, '');
const headings = text => [...stripCode(text).matchAll(/^(#{1,6}) (.+)$/gm)];
function anchors(text) {
  const counts = new Map();
  return new Set(headings(text).map(([, , title]) => {
    const base = title.toLowerCase().replace(/[^\p{L}\p{N}_\-\s]/gu, '').replace(/ /g, '-');
    const count = counts.get(base) ?? 0;
    counts.set(base, count + 1);
    return count ? `${base}-${count}` : base;
  }));
}
for (const [en, es] of pairs) {
  const [a, b] = await Promise.all([en, es].map(path => readFile(resolve(root, path), 'utf8')));
  if (headings(a).map(h => h[1]).join() !== headings(b).map(h => h[1]).join()) {
    errors.push(`Section structure differs: ${en} / ${es}`);
  }
  const expected = en === 'README.md' ? 'README.es.md' : `../es/${en.split('/').at(-1)}`;
  if (!a.includes(`](${expected})`)) errors.push(`Missing language link: ${en}`);
  const reverse = en === 'README.md' ? 'README.md' : `../en/${en.split('/').at(-1)}`;
  if (!b.includes(`](${reverse})`)) errors.push(`Missing language link: ${es}`);
}
for (const file of files) {
  const text = await readFile(resolve(root, file), 'utf8');
  if (headings(text).filter(h => h[1] === '#').length !== 1) errors.push(`${file}: expected one H1.`);
  if (/komikku-upscale-server/i.test(text)) errors.push(`${file}: obsolete project name.`);
  if (/\]\((?:https?:\/\/)?(?:TODO|TBD)/i.test(text)) errors.push(`${file}: placeholder link.`);
  for (const [, link] of stripCode(text).matchAll(/\[[^\]]*\]\(([^\s)]+)\)/g)) {
    if (/^(?:https?:|mailto:)/i.test(link)) continue;
    const [path, anchor] = link.split('#');
    const target = path ? resolve(root, dirname(file), decodeURIComponent(path)) : resolve(root, file);
    try {
      if (!(await stat(target)).isFile()) throw new Error('Not a file');
      if (anchor && target.endsWith('.md') && !anchors(await readFile(target, 'utf8')).has(decodeURIComponent(anchor))) {
        errors.push(`${file}: missing anchor ${link}`);
      }
    } catch {
      errors.push(`${file}: missing link target ${relative(root, target)}`);
    }
  }
}
if (errors.length) {
  console.error(errors.join('\n'));
  process.exitCode = 1;
} else console.log(`Checked ${files.length} documents: language pairs, headings and local links.`);
