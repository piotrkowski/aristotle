# Your first book

We will copy a DRM-free EPUB to the Kindle, read a page, and export a highlight. Aristotle must already be [installed](install.md). No agent, dictionary, or internet connection is needed.

Use an EPUB you may legally copy, such as a public-domain book. EPUB provides selectable text and the reader's full typography controls.

The steps use English interface labels. In a Polish interface, **Library** is **Biblioteka** and **Settings** is **Ustawienia**. The [screenshots](screenshots.md) show the controls.

## Copy the book

1. If a book is open in Aristotle, tap the top of the page and then **Library** to return to the library.
2. Hold **Library** for one second to return to the stock Kindle interface. Skip this step if Aristotle is not running.
3. Connect the Kindle to the computer with a USB cable.
4. If the optional agent keeps USB in network mode, choose **KUAL > Aristotle > USB drive mode for 10 minutes**.
5. Open the Kindle drive on the computer. Copy the EPUB into `documents/`.
6. Eject the drive before you disconnect the cable.

The book is now on the Kindle. Subfolders under `documents/` are allowed.

## Open and turn a page

1. Choose **KUAL > Aristotle > Start Aristotle**.
2. Open **Books > All** and tap the book. The library scans for files at startup.
3. Tap the right side in the lower half of the page to move forward. A new page appears.
4. Tap the left edge in the lower half to return to the previous page.

The top 12% opens the menu instead of turning a page. Below that area, the left 30% goes back and the rest goes forward. Horizontal swipes also turn pages.

## Change the text size

1. Tap the top of the page to open the reader menu.
2. Tap the larger **A** beside **Size** once. The text becomes larger.
3. Tap the book page between the top bar and bottom controls to close the menu. This tap does not turn the page.

The text is laid out again at your current position. Page numbers can take longer to appear because pagination runs in the background.

## Save a highlight

1. Hold a word for about half a second.
2. Drag across a short phrase on the same page.
3. Release your finger and choose **Highlight**.

The phrase remains highlighted. A dictionary is not required. You can review annotations through **Highlights** in the reader menu. There is no on-screen keyboard for writing notes.

By default, the highlight also appends to `documents/My Clippings.txt`. A Markdown file for the book is written under `aristotle-notes/` on the Kindle drive. Deleting a highlight later does not remove its old clippings entry.

## Finish and check the export

1. Tap the top of the page, then tap **Library** in the reader menu.
2. Hold **Library** for one second to quit Aristotle.
3. Wait for the stock interface to return. The boot splash can last 30 to 60 seconds [dev].
4. Connect the Kindle in USB drive mode and open `aristotle-notes/` on the computer.

The book's Markdown file contains the highlighted phrase. Next time, Aristotle resumes from its saved position.

The screensaver only turns off the light and displays a book card. It does not suspend the CPU, and the power button has no effect by default. Quit Aristotle when you finish reading. Battery use is unverified.

Next, [add a dictionary](dictionaries.md), [set up sync](network.md), or [back up your reading data](maintenance.md).
