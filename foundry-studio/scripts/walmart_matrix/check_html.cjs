// Verify both report formats in an isolated headless browser.
// Usage: node check_html.cjs [documents-directory] [qa-output-directory]
const {chromium} = require('playwright');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const {pathToFileURL} = require('node:url');
const documents = path.resolve(process.argv[2] || path.join(__dirname, '../../docs/easyloop/overnight-2026-09-14'));
const output = path.resolve(process.argv[3] || path.join(documents, 'qa'));

(async () => {
  fs.mkdirSync(output, {recursive:true});
  const browser = await chromium.launch({headless:true});
  try {
    const context = await browser.newContext({viewport:{width:1440,height:1000}, offline:true});
    const page = await context.newPage();
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    const fits = async () => assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > innerWidth), false, 'horizontal page overflow');
    await page.goto(pathToFileURL(path.join(documents, 'report.html')).href);
    const matrixRows = await page.locator('#matrix tbody tr').count();
    assert.ok(matrixRows > 0);
    await fits();
    await page.screenshot({path:path.join(output, 'report-desktop.png')});
    await page.setViewportSize({width:390,height:844});
    await fits();
    await page.screenshot({path:path.join(output, 'report-mobile.png')});

    await page.setViewportSize({width:1440,height:1000});
    await page.goto(pathToFileURL(path.join(documents, 'variants.html')).href);
    await page.waitForSelector('#throughput svg');
    const cases = await page.evaluate(() => DATA.cases.map(c => ({id:c.id,status:c.status})));
    assert.equal(cases.length, matrixRows);
    assert.equal(await page.locator('#variants a').count(), cases.length);
    const outcomes = [];
    for (const variant of cases) {
      await page.evaluate(id => location.hash = id, variant.id);
      await page.waitForFunction(id => currentId === id, variant.id);
      const charts = await page.locator('main svg').count();
      assert.equal(charts, variant.status === 'preflight' ? 0 : 4, variant.id);
      await fits();
      const values = await page.evaluate(() => {
        const c = byId.get(currentId);
        return c.series ? {
          expected:c.series.whole.picks_per_hour,
          shown:Number(document.querySelector('.kpi strong').textContent.replace(/,/g,'')),
        } : null;
      });
      if (values) assert.ok(Math.abs(values.shown-values.expected) <= .051, variant.id+' mean rate');
      outcomes.push({id:variant.id,charts});
    }
    const recorded = cases.find(c => c.status === 'recorded');
    assert.ok(recorded, 'at least one recorded variant is needed for chart QA');
    await page.evaluate(id => location.hash = id, recorded.id);
    await page.waitForFunction(id => currentId === id, recorded.id);
    await page.selectOption('#horizon', 'common');
    await page.selectOption('#period', '15');
    await page.selectOption('#scale', 'local');
    await page.locator('#waiting svg').focus();
    await page.keyboard.press('ArrowRight');
    assert.ok((await page.locator('#waiting .readout').innerText()).includes('HTM-minutes'));

    const csvEvent = page.waitForEvent('download');
    await page.click('#csv');
    await (await csvEvent).saveAs(path.join(output, 'chart-values.csv'));
    const rows = fs.readFileSync(path.join(output, 'chart-values.csv'),'utf8').trim().split('\n');
    assert.equal(rows.length, 1 + await page.locator('#values tbody tr').count());
    const jsonEvent = page.waitForEvent('download');
    await page.click('#download');
    await (await jsonEvent).saveAs(path.join(output, 'chart-data.json'));
    assert.deepEqual(JSON.parse(fs.readFileSync(path.join(output, 'chart-data.json'),'utf8')), await page.evaluate(() => DATA));
    await page.selectOption('#period', '60');
    await page.selectOption('#horizon', 'whole');
    await page.selectOption('#scale', 'shared');
    await page.screenshot({path:path.join(output, 'variants-desktop.png')});
    await page.setViewportSize({width:390,height:844});
    await fits();
    await page.screenshot({path:path.join(output, 'variants-mobile.png')});
    await page.locator('header a[href="report.html"]').click();
    assert.equal(page.url(), pathToFileURL(path.join(documents, 'report.html')).href);
    assert.deepEqual(errors, []);
    const result = {matrixRows,variants:outcomes,charts:outcomes.reduce((sum,c)=>sum+c.charts,0),errors,
      checks:'offline rendering; all variants; mean rates; controls; keyboard values; CSV/JSON downloads; report link; desktop/mobile width'};
    fs.writeFileSync(path.join(output,'html-validation.json'), JSON.stringify(result,null,2)+'\n');
    console.log(JSON.stringify({matrixRows,charts:result.charts,errors,output}));
  } finally {
    await browser.close();
  }
})().catch(error => { console.error(error); process.exitCode = 1; });
