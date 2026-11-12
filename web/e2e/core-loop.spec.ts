import { expect, test } from '@playwright/test'

// The main path of slices 1 and 2, through the real UI: create a site, answer the interview,
// approve the library, topic map and calendar, draft, review in the editor, publish, and read
// the post on the hosted blog.

const answers: Record<string, string> = {
  Product: 'Ledgerly sends invoices automatically when a project milestone is marked done. Agencies stop chasing payments by email because reminders go out on a schedule.',
  Audience: 'Small agencies with five to fifty people who bill clients by project. The agency owner usually does invoicing at night.',
  'Pain points': 'Customers tell us invoicing takes their agencies a full day every month. Late payments are the biggest cash flow problem for small agencies.',
  Positioning: 'Ledgerly is the invoicing tool for agencies that bill by milestone, not by the hour.',
  Opinions: 'We think time tracking is the wrong basis for agency invoicing. Agencies should bill for outcomes and milestones, never for hours.',
  Competitors: 'Buyers compare us with FreshBooks and Harvest for agency invoicing. Unlike Harvest, Ledgerly does not require time tracking before agencies can send invoices.',
  Stories: 'One design agency cut their invoicing time from eight hours to forty minutes a month after switching to Ledgerly.',
  Avoid: 'Never call the product cheap or promise guaranteed payment times to agency customers.',
}

test('from interview to a published post on the hosted blog', async ({ page }) => {
  const name = `Ledgerly ${Date.now().toString(36)}`

  await page.goto('/sites/new')
  await page.getByLabel('Product or company name').fill(name)
  await page.getByLabel('What it does').fill('Milestone invoicing for agencies.')
  await page.getByRole('button', { name: 'Create site and start the interview' }).click()
  await expect(page.getByRole('heading', { name: 'Interview' })).toBeVisible()

  // Answer the eight core questions. Follow-up questions may appear; answer those too.
  for (let i = 0; i < 12; i++) {
    const form = page.getByTestId('question').first()
    if (!(await form.isVisible().catch(() => false))) break
    const badge = (await form.locator('span').first().textContent()) ?? ''
    const answer = answers[badge.trim()] ?? 'A concrete example: one agency stopped losing a day a month to invoicing after switching.'
    await form.getByRole('textbox').fill(answer)
    await form.getByRole('button', { name: 'Save answer' }).click()
    await expect(page.getByText('Answer saved').first()).toBeVisible()
    await page.waitForTimeout(400)
  }
  await expect(page.getByText('All questions answered')).toBeVisible({ timeout: 30_000 })

  // Approve the proposed library entries.
  await page.getByRole('link', { name: 'Review proposed entries' }).click()
  await expect(page.getByTestId('entry').first()).toBeVisible({ timeout: 30_000 })
  await page.getByRole('button', { name: /Approve all/ }).click()
  await expect(page.getByText(/Approved \d+ entries/)).toBeVisible()

  // Propose and approve the topic map.
  await page.getByRole('link', { name: 'Topic map' }).click()
  await page.getByRole('button', { name: /Propose a topic map/ }).first().click()
  await expect(page.getByTestId('topic').first()).toBeVisible({ timeout: 30_000 })
  await page.getByRole('button', { name: /Approve all \d+/ }).click()
  await expect(page.getByText(/Approved \d+ topics/)).toBeVisible()

  // Plan and approve the calendar, then draft the first piece now.
  await page.getByRole('link', { name: 'Calendar' }).click()
  await page.getByRole('button', { name: /Plan the next three weeks|Plan next weeks/ }).first().click()
  await expect(page.getByTestId('piece').first()).toBeVisible({ timeout: 30_000 })
  await page.getByRole('button', { name: /Approve \d+ planned/ }).click()
  await expect(page.getByText(/Approved \d+ pieces/)).toBeVisible()
  await page.getByTestId('piece').first().getByRole('button', { name: 'Draft now' }).click()
  await expect(page.getByTestId('piece').first().getByRole('link', { name: 'Open draft' })).toBeVisible({ timeout: 45_000 })
  await page.getByTestId('piece').first().getByRole('link', { name: 'Open draft' }).click()

  // Review in the editor: citations show as chips and the structure checks run.
  const editor = page.getByRole('textbox', { name: 'Post body' })
  await expect(editor).toBeVisible()
  await expect(editor.locator('.citation-chip').first()).toBeVisible()
  await expect(page.getByTestId('findings')).toContainText('cite a library entry')
  const title = await page.getByLabel('Title', { exact: true }).inputValue()

  // Approve for immediate publishing.
  const now = new Date(Date.now() - 60_000)
  const local = new Date(now.getTime() - now.getTimezoneOffset() * 60_000).toISOString().slice(0, 16)
  await page.getByLabel('Publish at (optional)').fill(local)
  await page.getByRole('button', { name: 'Approve' }).click()
  await expect(page.getByText(/Approved\. Publishes/)).toBeVisible()
  await expect(page.getByRole('link', { name: 'View live' })).toBeVisible({ timeout: 45_000 })

  // The hosted blog serves it without citation markers.
  const live = await page.getByRole('link', { name: 'View live' }).getAttribute('href')
  expect(live).toBeTruthy()
  const resp = await page.goto(live!)
  expect(resp?.status()).toBe(200)
  await expect(page.getByRole('heading', { level: 1, name: title })).toBeVisible()
  expect(await page.content()).not.toContain('[[E')
})
