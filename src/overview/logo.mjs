// The Monocircuit logo of the title band, in half blocks: two bars, then the
// two blocks with the triangle between them (the SVG of the login pages,
// monocircuit-keycloak-theme/src/assets/img/monocircuit.svg).
//
// Drawn by hand, no longer rasterized from the SVG: square on screen it is 6
// columns × 3 lines (a cell is about twice as tall as wide), and at that size
// a rasterizer drops the triangle, it covers too little of its cells. The
// bottom line gives each shape a column of its own instead.

/** The logo, one string per line; every line the same width. */
export const LOGO = ["▀▀▀▀▀▀", "▀▀▀▀▀▀", "▀ ▀▀ ▀"];
/** How many lines the logo takes, the height of the title band. */
export const LOGO_LINES = LOGO.length;

/** The logo of the title band. */
export const titleLogo = () => LOGO;
