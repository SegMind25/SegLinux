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
  // The media query's change event arrives asynchronously: wait for it.
  await page.emulateMedia({reducedMotion:'reduce'});await page.waitForFunction(()=>document.getElementById('motion-toggle').disabled,null,{timeout:3000});
  await page.emulateMedia({reducedMotion:'no-preference'});

  // Animations, with motion enabled again.
  const fresh=await browser.newPage({viewport:{width:1280,height:900}});fresh.on('pageerror',e=>errors.push(e.message));
  await fresh.route('https://api.github.com/**',route=>route.fulfill({json:JSON.parse(fs.readFileSync(path.join(root,'releases.json')))}));
  await fresh.goto(url);
  // Revealed elements get their own transitions back: the guide cards lift on hover.
  await fresh.locator('#documentation').scrollIntoViewIfNeeded();
  const card=fresh.locator('.doc-card').first();
  await fresh.waitForFunction(()=>!document.querySelector('.doc-card.reveal-ready'),null,{timeout:3000});
  assert.match(await card.evaluate(e=>getComputedStyle(e).transitionProperty),/border-color/);
  await card.hover();await fresh.waitForTimeout(350);
  assert.notEqual(await card.evaluate(e=>getComputedStyle(e).transform),'none','guide card lifts on hover');
  await fresh.locator('#features').scrollIntoViewIfNeeded();
  await fresh.waitForFunction(()=>!document.querySelector('.feature.reveal-ready'),null,{timeout:3000});
  assert.match(await fresh.locator('.feature').first().evaluate(e=>getComputedStyle(e).transitionProperty),/background/);
  // The section in view is highlighted in the top bar, Download included.
  await fresh.evaluate(()=>{document.documentElement.style.scrollBehavior='auto';document.getElementById('download').scrollIntoView({block:'center'})});
  await fresh.waitForFunction(()=>document.querySelector('.nav-links .active')?.getAttribute('href')==='/download',null,{timeout:3000});
  // The scroll marker moves with a transform, and the orbit rests off screen.
  assert.equal(await fresh.locator('#ruler-marker').evaluate(e=>e.style.top),'');
  assert.match(await fresh.locator('#ruler-marker').evaluate(e=>e.style.transform),/translateY/);
  await fresh.waitForFunction(()=>getComputedStyle(document.querySelector('.orbit-ticks')).animationPlayState==='paused',null,{timeout:3000});
  await fresh.evaluate(()=>scrollTo(0,0));
  await fresh.waitForFunction(()=>getComputedStyle(document.querySelector('.orbit-ticks')).animationPlayState==='running',null,{timeout:3000});
  // A screenshot tab answers at once, even while its image downloads.
  let releaseShot;await fresh.route('**/assets/setup.webp',route=>new Promise(resolve=>{releaseShot=async()=>{await route.continue();resolve()}}));
  await fresh.locator('.tabs button[data-shot=setup]').click();
  assert.equal(await fresh.locator('.tabs button[data-shot=setup]').getAttribute('aria-selected'),'true');
  assert.equal(await fresh.locator('#screenshot-panel').getAttribute('aria-busy'),'true');
  await releaseShot();
  await fresh.waitForFunction(()=>document.querySelector('#shot').src.endsWith('/assets/setup.webp')&&!document.querySelector('#screenshot-panel').hasAttribute('aria-busy'));
  // The enlarged screenshot stays open on clicks inside it, closes outside.
  await fresh.locator('#shot').click();await fresh.locator('dialog[open]').waitFor();
  const box=await fresh.locator('dialog.lightbox').boundingBox();
  await fresh.mouse.click(box.x+4,box.y+box.height/2);assert.equal(await fresh.locator('dialog[open]').count(),1,'padding click keeps the screenshot open');
  await fresh.mouse.click(2,2);assert.equal(await fresh.locator('dialog[open]').count(),0,'backdrop click closes the screenshot');
  // Copy without clipboard access selects the command; the label resets once.
  await fresh.evaluate(()=>{Object.defineProperty(navigator,'clipboard',{value:{writeText:()=>Promise.reject(new Error('denied'))}})});
  const copy=fresh.locator('.code .copy').first();await copy.scrollIntoViewIfNeeded();
  await copy.click();await fresh.waitForTimeout(800);await copy.click();await fresh.waitForTimeout(1000);
  assert.equal((await copy.textContent()).trim(),'Selected','a second click restarts the delay');
  assert.equal(await fresh.evaluate(()=>getSelection().toString().trim()),'python3 download-full.py');
  await fresh.waitForTimeout(800);assert.equal((await copy.textContent()).trim(),'Copy');
  await fresh.close();

  assert.deepEqual(errors,[]);console.log('Website regression checks passed: navigation, gallery keyboard/focus, live motion, saved theme, download refresh, docs search, mobile overflow, reveal/hover transitions, section highlight, scroll marker, orbit pause, tab responsiveness, lightbox, copy fallback.');
 } finally {await browser.close();server.close();}
})().catch(e=>{console.error(e);process.exit(1)});
