// The overview in a browser (#435): the page asks the daemon for its view
// every two seconds, as wide as the window holds, and shows what it drew.
// Every URL is relative: nothing leaves the daemon, and it works behind a
// proxy's path too. A failed look says so and tries again.
const EVERY_MS = 2000;
const page = document.getElementById("page");

/** The columns the window holds in the frame's font. */
const columns = () => {
  const probe = document.createElement("pre");
  probe.className = "frame";
  probe.style.position = "absolute";
  probe.style.visibility = "hidden";
  probe.textContent = "x".repeat(100);
  document.body.append(probe);
  const width = probe.getBoundingClientRect().width / 100;
  probe.remove();
  return Math.max(
    40,
    Math.floor(document.documentElement.clientWidth / (width || 8)) - 1,
  );
};

const query = () => {
  const params = new URLSearchParams(location.search);
  params.set("width", String(columns()));
  return params.toString();
};

const look = async () => {
  try {
    const response = await fetch(`page?${query()}`, { cache: "no-store" });
    const text = await response.text();
    if (response.ok) {
      page.innerHTML = text;
    } else {
      page.innerHTML = "";
      const said = document.createElement("p");
      said.className = "quiet";
      said.textContent = `breadboard: ${text}`;
      page.append(said);
    }
  } catch {
    const said = document.createElement("p");
    said.className = "quiet";
    said.textContent = "breadboard serve doesn't answer: trying again";
    page.replaceChildren(said);
  }
  setTimeout(look, EVERY_MS);
};

// Writes (`[web] writes = true`): only from here, with the header a form of another page can't send.
page.addEventListener("click", async event => {
  const write = event.target.closest("[data-write]");
  if (!write) return;
  event.preventDefault();
  const params = new URLSearchParams(location.search);
  await fetch(`${write.dataset.write}?${params}`, {
    method: "POST",
    headers: { "x-breadboard": "write" },
  });
});

look();
