// Regenerates tests/fixtures/shape-reference.csv from the original TypeScript implementation.
//
//   git clone --depth 1 https://github.com/Dao-Ho/portfolio /tmp/dao-portfolio
//   bun scripts/gen-shape-reference.ts /tmp/dao-portfolio/src/app/components/koi-pond/shape.ts
//
// The Rust port must classify every sample the same way; tests/shape_fidelity.rs checks it.

import { classify } from "/tmp/dao-portfolio/src/app/components/koi-pond/shape.ts";

const out: string[] = ["s,a,past,flare,flag,across"];
const hit = { flag: 0, across: 1, du: 1, da: 0 };

for (let i = 0; i <= 40; i++) {
    const s = i / 40;
    for (let j = 0; j <= 20; j++) {
        const a = j * 0.01;
        for (const past of [0, 0.03]) {
            for (const flare of [0.8, 1, 1.2]) {
                classify(s, a, past, flare, hit);
                out.push(`${s.toFixed(6)},${a.toFixed(6)},${past},${flare},${hit.flag},${hit.across}`);
            }
        }
    }
}

console.log(out.join("\n"));
