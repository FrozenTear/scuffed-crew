// Seeds an empty local stack in order. Run once per fresh database.
for (const step of ['./seed.mjs', './seed2.mjs', './team.mjs', './relay.mjs', './seed-editor.mjs']) {
  console.log('==> ' + step);
  await import(step);
}
