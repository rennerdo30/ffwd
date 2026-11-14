# ffwd.renner.dev

The project homepage, served by GitHub Pages from this folder. It is plain static files: no build step, and no external fonts or scripts.

- **Download links:** `releases.js` reads the public releases API so the links point at the newest version. Without it, the links fall back to the releases page.
- **Screenshot viewer:** `site.js` switches the hero's screenshot between views and themes. Without it, the page shows the history in the visitor's colour scheme.
- **Screenshots:** `assets/shots/` holds real captures of the app (`ffwd --snapshot`) in the dark and light themes. [docs/SCREENSHOTS.md](../docs/SCREENSHOTS.md) says how to make them.
- **Speed figures:** the numbers in `index.html` come from [docs/SPEC.md](../docs/SPEC.md) §3. Update them together with their date and machine when they change.
- **Deploy:** run the **Website Pages** workflow from the Actions tab. It runs only when started by hand.
  - The repository's Pages source must be **GitHub Actions**.
  - The domain `ffwd.renner.dev` needs a CNAME record pointing to `rennerdo30.github.io`.
- **Rules:**
  - name no other products;
  - claim nothing the app does not do;
  - state where every number was measured.
