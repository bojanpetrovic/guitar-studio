// The user manual's screenshots, in order. Run by scripts/docs/screenshots.mjs (see
// `make docs-screenshots`) against a scratch data folder that holds only a plugins.json:
// it builds the demo presets first, then walks through the app.
//
// Only free or built-in plugins appear: Apple's AUs, Neural Amp Modeler, ParametricOD,
// GuitarML Proteus and MeldaProduction's free effects.

const K = {
  od: "au:aufx:5Z8c:SDAa",
  nam: "au:aufx:1YEo:SDAa",
  proteus: "au:aufx:Prt3:GtML",
  dly: "au:aufx:dely:appl",
  rvb: "au:aufx:rvb2:appl",
  comp: "au:aumf:MAe1:Meld",
  trem: "au:aumf:MAe7:Meld",
  flange: "au:aumf:MAE&:Meld",
  conv: "au:aumf:MCez:Meld",
};

export default async function tour(page) {
  await page.open();
  await setUp(page);

  // ------------------------------------------------------------ the rig
  await page.open();
  await page.key("1", ["Meta"]);
  await page.sleep(1000);
  await page.shot("overview");
  await page.shot("presets-panel", { selector: ".sidebar.left" });
  await page.shot("plugins-panel", { selector: ".sidebar.right" });
  await page.shot("dock", { selector: "footer.dock" });

  await page.key("p", ["Meta"]);
  await page.key("b", ["Meta"]);
  await page.sleep(400);
  await page.click(".react-flow__controls-fitview");
  await page.sleep(800);
  await page.shot("rig");
  await page.shot("scenes", { selector: ".scenebar", pad: 8 });
  await page.shot("mixer", { selector: ".react-flow__node", text: "Mixer", pad: 16 });

  await page.eval(() => {
    const node = [...document.querySelectorAll(".react-flow__node")].find((n) => n.textContent.startsWith("AUReverb2"));
    node.querySelector('button[title^="Replace"]').setAttribute("data-docs", "replace");
  });
  await page.click('[data-docs="replace"]');
  await page.sleep(600);
  await page.shot("replace", { selector: ".modal", pad: 24 });
  await page.key("Escape", [], "Escape");

  await page.click('[aria-label="Audio settings"]');
  await page.sleep(800);
  await page.shot("audio-settings", { selector: ".modal", pad: 24 });
  await page.key("Escape", [], "Escape");

  await page.click('[aria-label="Theme"]');
  await page.sleep(400);
  await page.shot("themes", { selector: ".theme-pop", pad: 0 });
  await page.click('[aria-label="Theme"]');

  await page.click('[aria-label="Phone remote"]');
  await page.sleep(500);
  await page.shot("phone-remote", { selector: ".phone-menu .popover", pad: 0 });
  await page.click('[aria-label="Phone remote"]');

  // ------------------------------------------------------------ the pedalboard
  await page.key("2", ["Meta"]);
  await page.sleep(800);
  await page.key("e", ["Meta"]); // the board opens on stage; ⌘E edits it
  await page.sleep(500);
  await page.shot("board-edit-surfaces");
  // Select the delay pedal (its name strip, below the controls on it).
  const pedal = await page.eval(() => {
    const el = [...document.querySelectorAll(".pb-widget.type-panel")].find((w) => w.textContent.includes("AUDELAY"));
    const r = el.getBoundingClientRect();
    return { x: r.x + r.width / 2, y: r.bottom - 12 };
  });
  await page.mouse("mouseMoved", pedal.x, pedal.y);
  await page.mouse("mousePressed", pedal.x, pedal.y);
  await page.mouse("mouseReleased", pedal.x, pedal.y);
  await page.sleep(600);
  await page.shot("board-edit");
  await page.shot("pedal-finishes", { selector: ".inspector" });

  // Play: the stage (the window shows only the board).
  await page.key("Escape", [], "Escape");
  await page.click(".pb-toolbar button", "Play");
  await page.sleep(1500);
  await page.mouse("mouseMoved", 720, 300);
  await page.sleep(400);
  await page.shot("stage");
  await page.shot("board", { selector: ".pb-board-frame" });

  // ------------------------------------------------------------ a light theme
  await page.open({ theme: "daylight" });
  await page.key("1", ["Meta"]);
  await page.sleep(1000);
  await page.shot("daylight");

  // ------------------------------------------------------------ the phone
  await page.open({ phone: true, width: 390, height: 844, scale: 3 });
  await page.sleep(2500);
  await page.shot("phone");
}

/** The demo presets: four small rigs and the main one, with scenes and a pedalboard. */
async function setUp(page) {
  const call = page.call;
  const add = async (key, x, y) => (await call("add_plugin_node", { key, x, y }))[0];
  const connect = async (pairs) => {
    for (const [from, to] of pairs) await call("connect", { from, to });
  };

  for (const key of [K.nam, K.proteus, K.dly, K.rvb]) await call("set_favorite", { key, favorite: true });

  // A rig is Input (1) → … → Main Out (2).
  const rig = async (name, keys) => {
    await call("new_preset");
    await call("disconnect", { from: 1, to: 2 });
    const ids = [1];
    let x = 260;
    for (const k of keys) {
      ids.push(await add(K[k], x, 140));
      x += 260;
    }
    await call("move_node", { id: 1, x: 0, y: 140 });
    await call("move_node", { id: 2, x, y: 140 });
    ids.push(2);
    await connect(ids.slice(1).map((id, i) => [ids[i], id]));
    await call("save_preset", { name, overwrite: true });
  };
  await rig("Clean Machine", ["comp", "nam", "rvb"]);
  await rig("Lead Tone", ["od", "proteus", "dly"]);
  await rig("Ambient Swells", ["trem", "dly", "conv"]);
  await rig("Funk Chorus", ["comp", "flange", "nam"]);

  // The main rig: drive → amp, then delay and reverb in parallel into a mixer.
  await call("new_preset");
  await call("disconnect", { from: 1, to: 2 });
  await call("move_node", { id: 1, x: 0, y: 150 });
  const od = await add(K.od, 250, 150);
  const amp = await add(K.nam, 500, 150);
  const dly = await add(K.dly, 760, 30);
  const rev = await add(K.rvb, 760, 270);
  const [mix] = await call("add_mixer_node", { x: 1020, y: 110 });
  await call("move_node", { id: 2, x: 1300, y: 150 });
  await connect([[1, od], [od, amp], [amp, dly], [amp, rev], [amp, mix], [dly, mix], [rev, mix], [mix, 2]]);
  await call("set_mixer_strip", { id: mix, source: dly, field: "gainDb", value: -6 });
  await call("set_mixer_strip", { id: mix, source: dly, field: "pan", value: -0.3 });
  await call("set_mixer_strip", { id: mix, source: rev, field: "gainDb", value: -9 });
  await call("set_mixer_strip", { id: mix, source: rev, field: "pan", value: 0.3 });
  // Scenes: Rhythm (no delay), Lead (everything), Ambient (no drive).
  await call("set_bypass", { id: dly, bypass: true });
  await call("add_scene", { name: "Rhythm" });
  await call("add_scene", { name: "Lead" });
  await call("set_bypass", { id: dly, bypass: false });
  await call("add_scene", { name: "Ambient" });
  await call("set_bypass", { id: od, bypass: true });
  await call("select_scene", { index: 1 });
  await call("save_preset", { name: "Crunch & Delay", overwrite: true });

  // Its pedalboard: "Build from rig" in the UI, then dressed up.
  await page.open();
  await page.key("2", ["Meta"]);
  await page.sleep(800);
  await page.click("button", "Build from rig");
  await page.sleep(1000);
  const board = (await call("get_studio")).pedalboard;
  const finishes = { PARAMETRICOD: "flake", NEURALAMPMODELER: "hammered", AUDELAY: "neon", AUREVERB2: "scope", MIXER: "brushed" };
  for (const w of board.widgets) if (w.type === "panel") w.options = { ...w.options, finish: finishes[w.label] ?? "classic" };
  board.surface = "walnut";
  let n = 0;
  const id = () => `docs${n++}`;
  board.widgets.push(
    { id: id(), type: "label", x: 40, y: 460, w: 420, h: 60, label: "CRUNCH & DELAY", color: "#ffa630", binding: { kind: "none" }, options: { fontSize: 34 } },
    { id: id(), type: "footswitch", x: 560, y: 420, w: 100, h: 130, label: "TUNER", color: "#3ddc97", binding: { kind: "tuner" }, options: {} },
    { id: id(), type: "footswitch", x: 690, y: 420, w: 100, h: 130, label: "CLICK", color: "#ffd84a", binding: { kind: "metronome" }, options: {} },
    { id: id(), type: "knob", x: 820, y: 430, w: 90, h: 120, label: "TEMPO", color: "#ffa630", binding: { kind: "bpm" }, options: {} },
    { id: id(), type: "fader", x: 1080, y: 400, w: 70, h: 150, label: "MASTER", color: "#7aa2ff", binding: { kind: "level", node: 2 }, options: {} },
  );
  await call("set_pedalboard", { board });
  await call("save_preset", { name: "Crunch & Delay", overwrite: true });
}
