// Every agent of an orchestration has a person's name (#292), the one the
// user knows it by while it moves from issue to issue: `Mira Albrecht`. It
// is only shown; `claude --name` and SendMessage keep the feature. The
// record keeps it as the worker's `displayName` from its first start on; an
// agent without one gets the name its conversation id picks (the id is
// random, so is the name), the first not taken in the orchestration.

/** First names, varied and international. */
export const FIRST = [
  "Mira",
  "Jonas",
  "Amara",
  "Kenji",
  "Lucía",
  "Tomasz",
  "Ingrid",
  "Rafael",
  "Noor",
  "Elias",
  "Saanvi",
  "Mateo",
  "Freya",
  "Kwame",
  "Yuki",
  "Henrik",
  "Leila",
  "Dario",
  "Anouk",
  "Tariq",
  "Sofia",
  "Malik",
  "Hana",
  "Oskar",
  "Zainab",
  "Luca",
  "Aiko",
  "Emil",
  "Priya",
  "Bastian",
  "Ines",
  "Kofi",
  "Marta",
  "Arjun",
  "Elif",
  "Niklas",
  "Chiara",
  "Omar",
  "Signe",
  "Diego",
  "Nadia",
  "Felix",
  "Ayumi",
  "Ruben",
  "Thandi",
  "Mikael",
  "Lina",
  "Joaquín",
  "Ama",
  "Viktor",
  "Mei",
  "Soren",
  "Fatima",
  "Pavel",
  "Clara",
  "Idris",
  "Astrid",
  "Rohan",
  "Valentina",
  "Jakob",
];
/** Last names, as varied. */
export const LAST = [
  "Albrecht",
  "Okafor",
  "Tanaka",
  "García",
  "Nowak",
  "Lindqvist",
  "Haddad",
  "Moreau",
  "Rossi",
  "Kowalczyk",
  "Mensah",
  "Sato",
  "Fischer",
  "Novák",
  "Petrov",
  "Ibrahim",
  "Jensen",
  "Costa",
  "Yilmaz",
  "Kim",
  "Bauer",
  "Silva",
  "Nakamura",
  "Dubois",
  "Andersen",
  "Mehta",
  "Herrera",
  "Schulz",
  "Osei",
  "Virtanen",
  "Rahman",
  "Bianchi",
  "Kovač",
  "Lehmann",
  "Adeyemi",
  "Ito",
  "Navarro",
  "Holm",
  "Farouk",
  "Wójcik",
  "Brandt",
  "Chen",
  "Duarte",
  "Eriksen",
  "Kaya",
  "Mbeki",
  "Lambert",
  "Horvat",
  "Keller",
  "Reyes",
  "Sørensen",
  "Abe",
  "Vogel",
  "Nkemelu",
  "Marino",
  "Šimek",
  "Weber",
  "Zhou",
  "Ortega",
  "Laine",
];

/** A number from `id`, the same every time (FNV-1a). */
const hashOf = id => {
  let hash = 0x811c9dc5;
  for (const char of id) {
    hash ^= char.codePointAt(0);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash;
};
/** The name in the place `n` of all FIRST × LAST, the first names varying first. */
const nameAt = n =>
  `${FIRST[n % FIRST.length]} ${LAST[Math.floor(n / FIRST.length) % LAST.length]}`;

/**
 * The name the conversation `id` picks, none of `taken`: from the place its
 * id points to on, the first whose first name no one has yet, else the
 * first not taken whole.
 */
export const pickName = (id, taken = []) => {
  const names = new Set(taken);
  const firsts = new Set([...names].map(name => name.split(" ")[0]));
  const all = FIRST.length * LAST.length;
  const start = hashOf(id) % all;
  let free = null;
  for (let step = 0; step < all; step++) {
    const name = nameAt(start + step);
    if (names.has(name)) continue;
    if (!firsts.has(name.split(" ")[0])) return name;
    free ??= name;
  }
  return free ?? nameAt(start);
};

/**
 * The name of every agent, by its conversation id: those of the record's
 * workers (`known`, by id) that have one keep it, the others (and each of
 * `ids` the record doesn't know) pick theirs in the order of their ids.
 */
export const displayNamesOf = (known = {}, ids = []) => {
  const names = new Map(
    Object.entries(known)
      .filter(([, entry]) => entry?.displayName)
      .map(([id, entry]) => [id, entry.displayName]),
  );
  const others = [...new Set([...Object.keys(known), ...ids])]
    .filter(id => id && !names.has(id))
    .toSorted();
  for (const id of others) names.set(id, pickName(id, names.values()));
  return names;
};
/** The name of the agent `id` (displayNamesOf), for its first start: kept from then on. */
export const displayNameOf = (known, id) => displayNamesOf(known, [id]).get(id);
