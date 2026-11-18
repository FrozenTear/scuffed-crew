// Crop the top of a full-page shot so it can be viewed at full size.
import { chromium } from 'playwright';
import fs from 'node:fs';
const [src, dst, h] = process.argv.slice(2);
const b = await chromium.launch(); const p = await b.newPage({ viewport: { width: 800, height: Number(h || 900) } });
await p.setContent(`<body style="margin:0;background:#888"><img src="data:image/png;base64,${fs.readFileSync(src).toString('base64')}" style="display:block;max-width:800px"></body>`);
await p.screenshot({ path: dst }); await b.close();
