import {readFile, readdir} from 'node:fs/promises';
import {resolve, join} from 'node:path';

/** Inventory only packages actually reached by esbuild, including nested
 * versions. Preserve upstream notice text; never invent missing copyrights. */
export async function thirdPartyNotices(inputs, directory) {
  const packages = new Map();
  for (const input of inputs) {
    if (input.startsWith('(') || !input.includes('node_modules/')) continue;
    const matches = [...input.matchAll(/(?:^|\/)node_modules\/((?:@[^/]+\/)?[^/]+)/g)];
    if (!matches.length) continue;
    const match = matches.at(-1), path = resolve(directory, input.slice(0, match.index + match[0].length));
    const metadata = JSON.parse(await readFile(join(path, 'package.json'), 'utf8'));
    if (!metadata.name || !metadata.version) throw Error('Unidentified bundled dependency');
    const identity = `${metadata.name}@${metadata.version}`;
    if (packages.has(identity)) continue;
    const files = (await readdir(path, {withFileTypes: true})).filter(f => f.isFile() && /^(licen[cs]e|copying|notice|copyright)([.-]|$)/i.test(f.name)).map(f => f.name).sort();
    const notices = [];
    for (const file of files) notices.push({file, text: await readFile(join(path, file), 'utf8')});
    if (!notices.length) {
      // Some packages put their complete license in README rather than LICENSE.
      for (const file of ['README.md', 'readme.md', 'README.markdown']) {
        let text; try { text = await readFile(join(path, file), 'utf8'); }
        catch (e) { if (e.code === 'ENOENT') continue; throw e; }
        const section = text.match(/^#{1,3}\s+licen[cs]e\s*\r?\n([\s\S]*)/im);
        if (section) notices.push({file: `${file} (license section)`, text: section[1].trim()});
        break;
      }
    }
    const fullTextPresent = notices.some(n => /permission is hereby granted|permission to use, copy|free and unencumbered software|redistribution and use|gnu (lesser )?general public license|apache license|mozilla public license|isc license/i.test(n.text));
    packages.set(identity, {identity, name: metadata.name, version: metadata.version, license: metadata.license ?? null,
      repository: metadata.repository?.url ?? metadata.repository ?? null, notices, full_text_present: fullTextPresent});
  }
  const rows = [...packages.values()].sort((a,b) => a.identity.localeCompare(b.identity));
  const unresolved = rows.filter(p => !p.full_text_present).map(p => p.identity);
  const text = ['Local Wallet private engine — bundled third-party notices',
    'Generated from the exact esbuild input graph. Upstream terms remain applicable.',
    'This file does not replace GPL corresponding-source or other distribution requirements.',
    ...(unresolved.length ? [`Upstream full-text notices still requiring distribution review: ${unresolved.join(', ')}`] : []),
    '', ...rows.flatMap(p => ['='.repeat(72), p.identity, `Declared license: ${p.license ?? 'not supplied'}`,
      `Upstream: ${p.repository ?? 'not supplied'}`, ...p.notices.flatMap(n => ['', `Source: ${n.file}`, n.text]), ''])].join('\n') + '\n';
  return {text, inventory: {schema_version: 1, packages: rows.map(({notices,...p}) => ({...p, notice_files: notices.map(n => n.file)})),
    missing_full_notice_review: unresolved, distribution_approved: false}};
}
