// The Docs section in the browser: the side rail's Docs entry opening the list and a document from it; on the
// live-frames document, its figures drawn at their own size and in the theme the person chose, a link to a heading
// bringing it into view with its address and moving focus to it, links to another document and to a heading of another
// document opening them here, that heading taking focus; and axe finding no serious or critical violation, light and dark. It needs no site, conversation or run.

import AxeBuilder from '@axe-core/playwright';
import { expect, test, type Locator, type Page } from '@playwright/test';

const LIVE_FRAMES = '/docs/docs/architecture/live-frames';

/** The mean of the red, green and blue of a pixel in the figure's empty left margin, as the screen shows it. */
async function marginShade(page: Page, figure: Locator): Promise<number> {
  const shot = await figure.screenshot();
  return page.evaluate(async (png) => {
    const image = new Image();
    image.src = `data:image/png;base64,${png}`;
    await image.decode();
    const canvas = document.createElement('canvas');
    canvas.width = image.width;
    canvas.height = image.height;
    const context = canvas.getContext('2d');
    context?.drawImage(image, 0, 0);
    const [red, green, blue] = context?.getImageData(8, Math.floor(image.height / 2), 1, 1).data ?? [];
    return (red + green + blue) / 3;
  }, shot.toString('base64'));
}

test('the side rail\'s Docs entry opens the list, and the list opens a document', async ({ page }) => {
  await page.goto('/');
  await page.getByRole('navigation', { name: 'Sections' }).getByRole('link', { name: 'Docs' }).click();
  await expect(page).toHaveURL(/\/docs$/);
  await page.getByRole('region', { name: 'docs/architecture/' }).getByRole('link', { name: 'How a run\'s picture reaches the browser' }).click();
  await expect(page).toHaveURL(new RegExp(`${LIVE_FRAMES}$`));
  await expect(page.getByRole('heading', { name: 'How a run\'s picture reaches the browser' })).toBeVisible();
});

test('the live-frames document draws its figures at their own size, in the theme the person chose', async ({ page }) => {
  await page.emulateMedia({ colorScheme: 'light' });
  await page.goto(LIVE_FRAMES);
  const figure = page.getByRole('img', { name: /^The life of a frame/ });
  await expect(figure).toBeVisible();
  const size = await figure.evaluate((image: HTMLImageElement) => ({
    natural: image.naturalWidth, shown: image.getBoundingClientRect().width,
  }));
  expect(size.natural).toBeGreaterThan(0);
  expect(size.shown).toBe(size.natural);
  expect(await marginShade(page, figure)).toBeGreaterThan(200);

  // Settings' Dark against a light system: the figure follows the page, not the system.
  await page.evaluate(() => localStorage.setItem('agent-studio.theme', 'dark'));
  await page.reload();
  await expect(figure).toBeVisible();
  expect(await marginShade(page, figure)).toBeLessThan(60);
});

test('a link to a heading brings it into view with its address and focuses it; links to another document and its headings open them here', async ({ page }) => {
  await page.goto(LIVE_FRAMES);
  await page.getByRole('link', { name: 'the dated section at the end' }).click();
  await expect(page).toHaveURL(/#measured-on-the-dev-stack-29-october-2026$/);
  const dated = page.getByRole('heading', { name: 'Measured on the dev stack, 29 October 2026' });
  await expect(dated).toBeInViewport();
  await expect(dated).toBeFocused();

  await page.goto(`${LIVE_FRAMES}#why-a-frame-never-changes-once-sent`);
  const why = page.getByRole('heading', { name: 'Why a frame never changes once sent' });
  await expect(why).toBeInViewport();
  await expect(why).toBeFocused();

  await page.getByRole('link', { name: 'a global barrier' }).click();
  await expect(page).toHaveURL(/\/docs\/sim\/dessim\/docs\/global-barrier$/);
  await expect(page.getByRole('heading', { name: 'Global barriers' })).toBeVisible();

  await page.goBack();
  await page.getByRole('link', { name: 'section on slices' }).click();
  await expect(page).toHaveURL(/\/docs\/docs\/designs\/worldview-format#4-slices$/);
  const slices = page.getByRole('heading', { name: '4. Slices' });
  await expect(slices).toBeInViewport();
  await expect(slices).toBeFocused();
});

test('axe finds no serious or critical violation on the list and the document, light and dark', async ({ page }) => {
  for (const scheme of ['light', 'dark'] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    for (const route of ['/docs', LIVE_FRAMES]) {
      await page.goto(route);
      await page.locator('.docs-index, .doc').first().waitFor();
      const results = await new AxeBuilder({ page }).analyze();
      const grave = results.violations.filter((violation) => violation.impact === 'serious' || violation.impact === 'critical')
        .map((violation) => `${violation.id}: ${violation.nodes.map((node) => node.target.join(' ')).slice(0, 3).join(' | ')}`);
      expect(grave, `${scheme} ${route}`).toEqual([]);
    }
  }
});
