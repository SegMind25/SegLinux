// Run with Playwright installed: node site/tests/browser.cjs
// SITE_DIR points to a built website; CHROMIUM_PATH optionally selects a browser.
const {chromium}=require('playwright');
const assert=require('node:assert/strict');
const fs=require('node:fs'),path=require('node:path'),http=require('node:http');
(async()=>{
 const root=process.env.SITE_DIR||path.resolve('_site');
 const server=http.createServer((req,res)=>{let name=req.url.split('?')[0];if(name==='/download')name='/download.html';if(name==='/docs')name='/docs/';if(name.endsWith('/'))name+='index.html';const file=path.join(root,name);if(!fs.existsSync(file)){res.writeHead(404);return res.end();}res.setHeader('Content-Type',({'.html':'text/html','.css':'text/css','.js':'text/javascript','.json':'application/json','.webp':'image/webp','.svg':'image/svg+xml'})[path.extname(file)]||'text/plain');fs.createReadStream(file).pipe(res)});
 await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
 const url=`http://127.0.0.1:${server.address().port}`;
 const browser=await chromium.launch({headless:true,...(process.env.CHROMIUM_PATH?{executablePath:process.env.CHROMIUM_PATH}:{}),args:['--no-sandbox']});
 try {
  const page=await browser.newPage({viewport:{width:1280,height:900}});const errors=[];page.on('pageerror',e=>errors.push(e.message));
  // Hold the background refresh while the user opens the manual downloads.
  let releaseResponse;await page.route('https://api.github.com/**',route=>new Promise(resolve=>{releaseResponse=async()=>{await route.fulfill({json:JSON.parse(fs.readFileSync(path.join(root,'releases.json')))});resolve()}}));
  await page.goto(url+'/download');await page.getByText('Or download the parts manually').click();
  assert.equal((await page.locator('.nav-links .active').textContent()).trim(),'Download');
  assert.equal(await page.locator('details[open]').count(),1);
  await releaseResponse();await page.waitForTimeout(150);assert.equal(await page.locator('details[open]').count(),1);
  await page.unrouteAll({behavior:'wait'});await page.route('https://api.github.com/**',route=>route.fulfill({json:JSON.parse(fs.readFileSync(path.join(root,'releases.json')))}));
  await page.goto(url);await page.locator('.tabs button').first().focus();await page.keyboard.press('ArrowRight');
  await page.waitForFunction(()=>document.querySelector('#shot').src.endsWith('/assets/desktop.webp'));
  assert.equal((await page.locator('.tabs button[aria-selected=true]').textContent()).trim(),'Desktop');
  await page.locator('#shot').click();await page.locator('dialog[open]').waitFor();await page.keyboard.press('Escape');assert.equal(await page.locator('#shot').evaluate(e=>e===document.activeElement),true);
  await page.locator('#motion-toggle').click();assert.equal(await page.locator('html').getAttribute('data-motion'),'reduced');
  assert.equal(await page.locator('.orbit-ticks').evaluate(e=>getComputedStyle(e).animationName),'none');
  await page.locator('#theme-toggle').click();await page.goto(url+'/docs/');assert.equal(await page.locator('html').getAttribute('data-theme'),'light');assert.equal(await page.locator('html').getAttribute('data-motion'),'reduced');
  await page.locator('.doc-search').fill('gaming');assert.equal(await page.locator('.docs-sidebar nav a:visible').count(),1);
  for(const width of [390,320]) {await page.setViewportSize({width,height:844});for(const route of ['/','/download','/docs/']) {await page.goto(url+route);const overflow=await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth);if(overflow)console.log(await page.evaluate(()=>[...document.querySelectorAll('body *')].filter(e=>{const r=e.getBoundingClientRect();return r.width&&r.right>innerWidth+1&&!e.closest('.table-wrap,pre')}).map(e=>({tag:e.tagName,cls:typeof e.className==='string'?e.className:'svg',width:e.getBoundingClientRect().width,right:e.getBoundingClientRect().right})).slice(0,30)));assert.equal(overflow,false,`Horizontal overflow: ${route} at ${width}`);}}
  await page.goto(url);await page.locator('#menu-toggle').click();assert.equal(await page.locator('#menu-toggle').getAttribute('aria-expanded'),'true');await page.keyboard.press('Escape');assert.equal(await page.locator('#menu-toggle').getAttribute('aria-expanded'),'false');
  await page.emulateMedia({reducedMotion:'reduce'});assert.equal(await page.locator('#motion-toggle').isDisabled(),true);
  assert.deepEqual(errors,[]);console.log('Website regression checks passed: navigation, gallery keyboard/focus, live motion, saved theme, download refresh, docs search, mobile overflow.');
 } finally {await browser.close();server.close();}
})().catch(e=>{console.error(e);process.exit(1)});
