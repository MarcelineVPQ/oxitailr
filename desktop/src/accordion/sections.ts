// Every accordion page and its sections, in order: the one registry deep
// links are typed against (revealSection(page, id) only takes an id listed
// here). The accordion holds settings and actions only; readouts are
// main-view blocks (MAIN_BLOCKS, MainBlock.tsx). The DS5 configurator's
// framework, with oxitailr's pages.
export const SECTIONS = {
  app: ["appearance", "startup", "passwords", "config"],
  about: ["guide", "changes", "keys", "coming", "credits"],
} as const;

// The readouts in each page's main view (MainBlock.tsx).
export const MAIN_BLOCKS = {
  app: ["configFile", "passwordStore"],
} as const;

export type AccordionPage = keyof typeof SECTIONS;
export type BlockPage = keyof typeof MAIN_BLOCKS;
export type BlockId<P extends BlockPage> = (typeof MAIN_BLOCKS)[P][number];
export type SectionId<P extends AccordionPage> = (typeof SECTIONS)[P][number];
