---
name: Aristotle homepage
description: A personal post about a weekend Kindle project, with a clickable demo.
colors:
  paper: "#ffffff"
  ink: "#252525"
  muted: "#6b6b6b"
  rule: "#e4e4e4"
  focus: "#365c82"
  selection: "#e3e9f0"
  screen-edge: "#d2d2d2"
  dark-paper: "#191919"
  dark-ink: "#e6e6e6"
  dark-muted: "#a0a0a0"
  dark-rule: "#383838"
  dark-focus: "#a9c7e6"
  dark-selection: "#344353"
  dark-screen-edge: "#505050"
typography:
  body:
    fontFamily: "Inter, sans-serif"
    fontSize: "1rem"
    fontWeight: 400
    lineHeight: 1.8
  title:
    fontFamily: "Inter, sans-serif"
    fontSize: "clamp(1.75rem, 4.5vw, 2.125rem)"
    fontWeight: 500
    lineHeight: 1.3
    letterSpacing: "-0.035em"
  metadata:
    fontFamily: "Inter, sans-serif"
    fontSize: "0.8125rem"
  demo-controls:
    fontFamily: "Inter, sans-serif"
    fontSize: "0.8125rem"
    lineHeight: 1.6
  demo-caption:
    fontFamily: "Inter, sans-serif"
    fontSize: "0.8125rem"
    lineHeight: 1.6
rounded:
  hotspot: "2px"
  button: "3px"
  device: "5px"
spacing:
  paragraph-after: "1.4rem"
  page-gutter: "1.5rem"
  demo-control-gap: "1rem"
---

# Design system: Aristotle homepage

## Overview

This document covers `index.html` and `assets/post.css`, not the native Kindle interface or repository documentation.

The homepage reads as a dated personal blog post about a weekend project. Keep the author's informal English and first-person account. Fix typos without turning it into release copy. Inter and the restrained writing layout take their cue from the article the user supplied on jjmaxwell4.com.

## Colors

Light is the default. The manual theme toggle switches the page palette through `data-theme="dark"`. Use muted text for metadata, captions, and the footer. Links inherit the surrounding text color and remain underlined in the article.

Screenshot pixels retain their original colors in both page themes. The screen background stays white. Hotspots use a subtle `rgb(128 128 128 / 0.1)` hover fill and a fixed blue focus outline.

## Typography

Load the local variable Inter font. Use regular body text, a medium-weight title, and smaller metadata. The title has a `max-width` of `26ch` and balanced wrapping. Paragraphs use a 1.8 line height.

## Layout

Use one centered column, capped at 40rem, with 1.5rem side gutters. Place an icon-only GitHub link and theme toggle above the title and date. Keep the demo inside the article, between paragraphs.

The demo is capped at 22rem and keeps the screenshots' 1072 / 1448 aspect ratio. Enlarge widens it to 40rem, within the viewport. At widths up to 480px, reduce header padding and the spacing around the post header and demo. Metadata and footer links may wrap.

## Elevation & Depth

The page has no shadows, gradients, or raised cards. A thin screen border and footer divider provide separation.

## Shapes

Use the existing small radii for buttons, hotspots, and the screenshot frame. Do not add a decorative device bezel around the demo.

## Components

The demo uses real screenshots and clickable hotspots. Put the current instruction above the screenshot. The instruction is also a button that performs the suggested action. After a dictionary lookup, suggest the reader menu instead of another lookup. Keep keyboard focus on the instruction after keyboard navigation.

Greek text, English text, and Footnotes select separate reading samples, not consecutive pages. Continue reading returns to the last Greek or English sample. Menu and night-mode controls work in both themes. Reset returns to the initial library and clears the visited actions.

Show actions marks the available hotspots. Tapping an inactive area also reveals them and explains the preview's limits. Hotspots and controls have a minimum 44px touch target. Keep the caption readable at 0.8125rem. Link More screenshots to the GitHub screenshot gallery.

Header icons have accessible names and 44px touch targets. A hand-drawn arrow points at Continue reading on the initial demo screen. Hide it after opening a book, on screens up to 600px wide, and in the enlarged view. The footer holds only repository links.

## Do's and Don'ts

- Keep the date and personal weekend-project story without a separate project label.
- Preserve the author's wording except for typo and grammar fixes.
- Keep light as the default and offer a manual dark theme.
- Label the demo as a clickable screenshot preview, not a live device emulator.
- Do not add marketing cards, onboarding step counters, or red hotspot frames.
- Do not apply this document to the native Kindle interface.
- Keep the site to one HTML page. Link to repository documentation instead of separate guide pages.
