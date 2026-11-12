#!/usr/bin/env python3
"""setup_neon_pal.py - run from the folder that contains public/:  python setup_neon_pal.py"""
import glob, itertools, json, os, random, sys

if not os.path.isdir("public"):
    sys.exit("Run this from the folder that contains public/ (your inner paypal folder).")
os.makedirs("dataset", exist_ok=True)
random.seed(42)

# ---------------------------------------------------------------- dataset
SYSTEM = (
    "You are the PayPal Boardroom AI. For every brief, respond ONLY with a JSON object "
    'containing four takes, one per board seat: "Strategist", "Marketer", "CFO", and '
    '"Growth Lead". Each take is 2-4 sentences, money-aware, and focused on helping PayPal '
    'win and keep customers globally. End with a "recommendation" field: one sentence the '
    "board agrees on."
)

def cap(s):
    return s[0].upper() + s[1:]

def row(q, strategist, marketer, cfo, growth, rec):
    takes = {"Strategist": strategist, "Marketer": marketer, "CFO": cfo,
             "Growth Lead": growth, "recommendation": rec}
    return {"messages": [
        {"role": "system", "content": SYSTEM},
        {"role": "user", "content": q},
        {"role": "assistant", "content": json.dumps(takes)},
    ]}

core = [
    ("What is the single highest-leverage move to grow PayPal's customer base globally this year?",
     "Double down on cross-border commerce for small sellers in high-growth corridors across Southeast Asia, Latin America, Africa, and India. Every seller we onboard brings buyers with them, so one acquisition feeds both sides of the network.",
     "Sell one promise: get paid from anywhere, safely. Use seller communities and local creators as the channel, and let buyer protection carry the trust message.",
     "Cross-border transactions earn FX revenue and a higher take rate than domestic volume, so this growth is margin-accretive. I want CAC payback and cross-border share of volume reported by corridor.",
     "Pick three corridors, set a 30-day seller activation target, and run weekly onboarding experiments. Use licensed local partners so licensing delays do not stall the launch.",
     "Make cross-border selling for small businesses the lead growth bet this year."),
    ("How should PayPal compete with local wallets like GCash, Pix, UPI, and M-Pesa?",
     "Do not fight them head-on. Become the global rail that connects to them, as a funding source and a payout destination, so competitors turn into distribution.",
     "Co-brand with the local wallet where it is loved and keep the PayPal button and buyer protection visible in the flow. The message is local wallet at home, PayPal for the world.",
     "Domestic wallet volume runs at near-zero margin, so winning it would cost capital for little return. Partner integrations acquire users far cheaper than paid media and keep the cross-border margin with us.",
     "Ship one wallet integration per quarter and measure inbound volume and cost per user acquired through partners. Watch for commoditization and protect branded checkout placement in every deal.",
     "Integrate with local wallets instead of competing with them on domestic payments."),
    ("Should PayPal cut fees to win market share?",
     "No broad cuts. Use targeted, time-bound pricing in strategic corridors and for merchants who commit volume.",
     "Frame every concession as a launch promotion tied to a clear condition, never as a new everyday price. Promotional framing protects the perception of value.",
     "An across-the-board cut gives away margin on volume we would have kept anyway. Targeted pricing buys growth only where elasticity is high, and I want incremental volume per dollar of concession tracked.",
     "Test reduced cross-border fees for new sellers in one launch market for twelve months, with a control group. Scale only if the cohort stays profitable after the promotion ends.",
     "Use targeted promotional pricing in select corridors and keep headline fees intact."),
    ("Give me a marketing strategy to acquire Gen Z users globally.",
     "Lead with social payments and creator payouts outside the US, bundled with a lightweight account that is quick to open. Creators bring their audiences with them, so acquisition compounds.",
     "Work through local creators and native formats, not corporate ads. Make splitting, tipping, and getting paid feel social, with safety as the quiet proof point.",
     "Gen Z revenue per user is low today, so I will fund this only with strict CAC ceilings and retention gates. The case rests on lifetime, so 30 and 90 day retention matter more than sign-ups.",
     "Launch referral rewards paid after the first real transaction and measure the referral coefficient weekly. Iterate the product fast, because this audience churns to newer apps quickly.",
     "Acquire Gen Z through creator payouts and referrals, with retention-gated spend."),
    ("How do we make merchants choose PayPal over Stripe and Adyen?",
     "Compete on consumer demand, not on APIs. We bring a huge base of ready-to-pay users and a two-sided network, which pure processors cannot match.",
     "Give sales teams conversion proof from A/B tests, merchant by merchant. Speak to both audiences: revenue lift for the business owner and clean docs for the developer.",
     "Match competitors on card processing price where we must and hold margin on branded checkout, where we are differentiated. Track enterprise win rate and branded share of checkout.",
     "Close the developer experience gap with better SDKs, sandbox, and documentation, since technical teams can veto the deal. Offer a single integration that covers PayPal, cards, and local methods.",
     "Win merchants on proven conversion lift and a single integration, not on price alone."),
    ("How do we reduce churn among existing consumers?",
     "Move users from checkout-only to multi-product: balance, card, and at least one recurring payment. Customers with several products leave far less often.",
     "Build a habit loop with rewards people actually notice and reminders around bills and subscriptions. Celebrate small wins in the app so the brand shows up between purchases.",
     "Fund rewards from interchange and merchant co-funding so the program pays for itself. Cap reward cost per retained user and tie tiers to behaviors that earn margin.",
     "Run cohort tests on the first 60 days after sign-up and find the two actions that best predict retention. Then redesign onboarding to drive those actions first.",
     "Cut churn by moving checkout-only users to multiple products with self-funded rewards."),
]

regions = ["Southeast Asia", "Latin America", "Sub-Saharan Africa", "the Middle East",
           "Western Europe", "Eastern Europe", "Japan", "South Korea", "Australia", "Canada",
           "the United Kingdom", "Mexico", "Brazil", "Nigeria", "Indonesia", "Vietnam",
           "the Philippines", "Germany", "France", "Turkey"]
segments = ["freelancers", "small online sellers", "enterprise merchants", "students",
            "cross-border shoppers", "subscription businesses", "gig-economy workers",
            "nonprofits", "travel companies", "gaming and digital goods merchants",
            "creators and influencers", "B2B exporters"]
levers = ["referral programs", "buy-now-pay-later", "cashback rewards", "merchant lending",
          "FX pricing", "one-tap checkout", "stablecoin settlement", "telco partnerships",
          "marketplace integrations", "localized customer support", "a debit card",
          "invoicing tools"]

def gen_region(r):
    q = random.choice([
        f"What is our best go-to-market strategy for {r}?",
        f"How do we win more customers in {r}?",
        f"Give me a board-level growth plan for {r}.",
        f"Should we expand in {r}, and how?",
    ])
    return row(q,
        f"Enter {r} through cross-border receiving for local sellers and freelancers before touching domestic payments, where local players are entrenched. Partner with a licensed local institution so we launch in months, not years.",
        f"Lead with trust and getting paid from anywhere, using local-language onboarding and local seller communities as the channel. Every campaign in {r} should map to one funnel step.",
        f"Cross-border flows earn FX revenue and higher take rates than domestic volume in {r}, and each seller brings buyers at low incremental cost. I want CAC payback under twelve months before we scale spend.",
        f"Run a 90-day pilot in {r} with one payout partner and one seller community. Track activation within 30 days and cross-border volume per account, with a kill criterion set up front.",
        f"Launch a cross-border-first pilot in {r} with a licensed local partner.")

def gen_segment(s):
    q = random.choice([
        f"How should PayPal acquire and retain {s}?",
        f"What is the business case for targeting {s}?",
        f"Design a strategy to grow PayPal's share among {s} globally.",
    ])
    return row(q,
        f"Build a tailored bundle for {s}: fast onboarding, pricing that rewards volume, and one or two add-on products that solve their biggest pain point. Keep the core platform shared so we do not fragment the product.",
        f"Give {s} their own landing experience and messaging in their language, with proof from people like them. Recruit through the communities and platforms they already use.",
        f"{cap(s)} are worth pursuing only if product attach and retention hold up, because that is where the margin lives. Report revenue per active account and twelve-month retention for the segment.",
        f"Measure time to first transaction for {s} and remove every step that slows it. Test two bundles against each other and keep the one with the higher attach rate.",
        f"Target {s} with a dedicated bundle and judge it on attach rate and retention.")

def gen_lever(l):
    q = random.choice([
        f"Should PayPal invest more in {l}? Make the case.",
        f"What is the ROI argument for {l}?",
        f"How would {l} help PayPal gather more customers globally?",
    ])
    return row(q,
        f"Yes, invest in {l}, but gate the rollout by market. Start where the lever has the most pull, prove it, then expand.",
        f"Make {l} a headline benefit in the markets where it matters most, and explain it in one sentence a customer would repeat. Pair it with the trust message so it reads as PayPal, not a gimmick.",
        f"{cap(l)} must lower acquisition cost, raise conversion, or lift revenue per account. I will back it if incremental margin covers the cost within 12 to 18 months.",
        f"Pilot {l} in two or three markets for 90 days with a control group. Track cost per incremental account and set the kill criterion before we spend.",
        f"Invest in {l} through a gated pilot with clear payback targets.")

def gen_combo(l, s, r):
    return row(f"How can PayPal use {l} to attract {s} in {r}?",
        f"Offer {l} to {s} in {r} as part of the standard account, not a separate product. Secure the local partner and compliance sign-off first.",
        f"Tell {s} in {r} exactly what {l} does for them, in local language and through the channels they trust. Use early adopters as the proof.",
        f"Cap the downside with a six-month pilot budget and compare cost per acquired account against the regional average. Scale only when the unit economics are proven.",
        f"Set a volume target for the pilot and track activation, incremental volume, and 90-day retention for {s}. Review at the midpoint and adjust the offer.",
        f"Pilot {l} for {s} in {r} for six months with a firm budget and volume target.")

rows = [row(*c) for c in core]
rows += [gen_region(r) for r in regions]
rows += [gen_segment(s) for s in segments]
rows += [gen_lever(l) for l in levers]
rows += [gen_combo(*c) for c in random.sample(list(itertools.product(levers, segments, regions)), 60)]
random.shuffle(rows)

for path in ("dataset/paypal_boardroom_dataset.jsonl", "public/boardroom_dataset.jsonl"):
    with open(path, "w") as f:
        for r_ in rows:
            f.write(json.dumps(r_) + "\n")

# ---------------------------------------------------------------- Hugging Face files
README = r'''---
license: mit
task_categories:
- text-generation
language:
- en
tags:
- fintech
- strategy
- boardroom
- hackathon
---
# PayPal Boardroom AI Dataset

Chat-format JSONL (system / user / assistant). Each assistant reply is a JSON object with
four board-seat takes (Strategist, Marketer, CFO, Growth Lead) plus a one-line recommendation.
Synthetic, written for a hackathon demo. Not affiliated with PayPal.
'''

PUSH = r'''"""Push the dataset to Hugging Face:  python dataset/push_dataset.py"""
import os
from huggingface_hub import HfApi, login

login()  # paste a WRITE token from huggingface.co/settings/tokens
api = HfApi()
repo = api.whoami()["name"] + "/paypal-boardroom-ai"
api.create_repo(repo, repo_type="dataset", exist_ok=True)
here = os.path.dirname(os.path.abspath(__file__))
for name in ("paypal_boardroom_dataset.jsonl", "README.md"):
    api.upload_file(path_or_fileobj=os.path.join(here, name), path_in_repo=name,
                    repo_id=repo, repo_type="dataset")
print("Pushed: https://huggingface.co/datasets/" + repo)
'''

PULL = r'''"""Pull the dataset from Hugging Face:  python dataset/pull_dataset.py"""
import json, os
from datasets import load_dataset
from huggingface_hub import HfApi

repo = HfApi().whoami()["name"] + "/paypal-boardroom-ai"
ds = load_dataset(repo, split="train")
print(len(ds), "boardroom examples loaded from", repo)
takes = json.loads(ds[0]["messages"][2]["content"])
print("BRIEF:", ds[0]["messages"][1]["content"])
for seat in ("Strategist", "Marketer", "CFO", "Growth Lead"):
    print("\n[" + seat + "]", takes[seat])
here = os.path.dirname(os.path.abspath(__file__))
ds.to_json(os.path.join(here, "..", "public", "boardroom_dataset.jsonl"))
print("\nSaved to public/boardroom_dataset.jsonl")
'''

# ---------------------------------------------------------------- web files
HTML = r'''<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>Boardroom AI - Neon Pal</title>
  <link rel="stylesheet" href="boardroom-layout.css" />
  <script type="importmap">
    {"imports": {"three": "https://unpkg.com/three@0.160.0/build/three.module.js",
                 "three/addons/": "https://unpkg.com/three@0.160.0/examples/jsm/"}}
  </script>
</head>
<body>
  <header class="topbar">
    <a class="back-link" href="/">&larr; Workspace</a>
    <h1>Neon Pal Playbook</h1>
    <span id="np-status" class="status-pill">Loading...</span>
  </header>
  <main class="boardroom">
    <aside class="pal-column">
      <div id="neon-pal-stage" class="pal-stage"></div>
      <section class="neon-pal">
        <div id="neon-pal-log" class="neon-pal-log"></div>
        <form id="neon-pal-form" class="neon-pal-form">
          <input id="neon-pal-input" type="text" autocomplete="off"
                 placeholder="Ask the board: how do we win freelancers in Nigeria?" />
          <button type="submit">Ask</button>
        </form>
        <p class="np-note">Demo playbook: answers are matched by keyword from a synthetic dataset, not generated live. Not affiliated with PayPal.</p>
      </section>
    </aside>
    <section class="seats">
      <article class="seat"><h3>Strategist</h3><p id="np-take-strategist" class="take">Awaiting a brief...</p></article>
      <article class="seat"><h3>Marketer</h3><p id="np-take-marketer" class="take">Awaiting a brief...</p></article>
      <article class="seat"><h3>CFO</h3><p id="np-take-cfo" class="take">Awaiting a brief...</p></article>
      <article class="seat"><h3>Growth Lead</h3><p id="np-take-growth" class="take">Awaiting a brief...</p></article>
    </section>
  </main>
  <script src="neon-pal-chat.js"></script>
  <script type="module" src="neon-pal-avatar.js"></script>
</body>
</html>
'''

CSS = r''':root{--bg:#060912;--panel:#0b0f1a;--neon:#00d4ff;--pp:#0070ba;--text:#e6f7ff}
*{box-sizing:border-box}
body{margin:0;background:radial-gradient(circle at 30% 20%,#0f1a33,var(--bg) 60%);color:var(--text);font-family:system-ui,-apple-system,sans-serif}
.topbar{display:flex;align-items:center;gap:14px;padding:14px 24px;border-bottom:1px solid #16213a}
.topbar h1{font-size:20px;margin:0;flex:1}
.back-link{color:var(--neon);text-decoration:none;font-size:13px}
.np-note{margin:10px 0 0;font-size:12px;line-height:1.4;color:#8fa3c4}
.status-pill{font-size:12px;padding:6px 12px;border-radius:999px;border:1px solid var(--neon);color:var(--neon)}
.boardroom{display:grid;grid-template-columns:420px 1fr;gap:20px;padding:20px;min-height:calc(100vh - 66px)}
.pal-column{display:flex;flex-direction:column;gap:14px;min-width:0}
.pal-stage canvas{display:block;width:100%!important;height:100%!important}
.pal-stage{height:380px;border-radius:16px;background:linear-gradient(180deg,#0f1a33,#060912);border:1px solid var(--neon);box-shadow:0 0 30px rgba(0,212,255,.3);overflow:hidden}
.neon-pal{border:1px solid var(--neon);border-radius:14px;padding:14px;background:var(--panel);box-shadow:0 0 24px rgba(0,212,255,.25);flex:1;display:flex;flex-direction:column}
.neon-pal-log{flex:1;min-height:180px;max-height:300px;overflow-y:auto;margin-bottom:12px;padding:8px;background:var(--bg);border-radius:10px}
.np-msg{padding:8px 12px;border-radius:12px;margin:6px 0;max-width:88%;line-height:1.35;font-size:14px;width:fit-content}
.np-user{background:var(--pp);margin-left:auto}
.np-bot{background:#1a2238;border:1px solid var(--neon)}
.neon-pal-form{display:flex;gap:8px}
.neon-pal-form input{flex:1;padding:10px;border-radius:10px;border:1px solid #223;background:#0f1424;color:#fff}
.neon-pal-form button{padding:10px 16px;border-radius:10px;border:0;background:var(--neon);color:#000;font-weight:700;cursor:pointer}
.seats{display:grid;grid-template-columns:1fr 1fr;grid-auto-rows:1fr;gap:16px}
.seat{background:var(--panel);border:1px solid #1d2a4a;border-radius:16px;padding:18px}
.seat h3{margin:0 0 10px;color:var(--neon);font-size:14px;letter-spacing:.08em;text-transform:uppercase}
.take{margin:0;line-height:1.5;font-size:15px}
@media (max-width:900px){.boardroom{grid-template-columns:minmax(0,1fr)}.seats{grid-template-columns:1fr}}
'''

CHAT_JS = r'''"use strict";
// neon-pal-chat.js - loads boardroom_dataset.jsonl and fills the four board seats.
(() => {
  const $ = (s) => document.querySelector(s);
  const SLOTS = {
    Strategist: "#np-take-strategist",
    Marketer: "#np-take-marketer",
    CFO: "#np-take-cfo",
    "Growth Lead": "#np-take-growth",
  };
  let dataset = [];

  function setStatus(msg) {
    const el = $("#np-status");
    if (el) el.textContent = msg;
  }

  async function loadDataset() {
    const res = await fetch("boardroom_dataset.jsonl");
    if (!res.ok) throw new Error("dataset fetch failed: " + res.status);
    const text = await res.text();
    dataset = text.trim().split("\n").map((line) => {
      const row = JSON.parse(line);
      return { brief: row.messages[1].content, takes: JSON.parse(row.messages[2].content) };
    });
    setStatus("Neon Pal online - " + dataset.length + " plays loaded");
  }

  // Keyword-overlap lookup. Swap this for a model API call once you fine-tune.
  function findBest(query) {
    const words = new Set(query.toLowerCase().match(/[a-z0-9]+/g) || []);
    let best = null;
    let bestScore = 0;
    for (const row of dataset) {
      const bw = new Set(row.brief.toLowerCase().match(/[a-z0-9]+/g) || []);
      let score = 0;
      bw.forEach((w) => { if (w.length > 2 && words.has(w)) score += 1; });
      if (score > bestScore) { bestScore = score; best = row; }
    }
    return best || dataset[Math.floor(Math.random() * dataset.length)];
  }

  function bubble(role, text) {
    const log = $("#neon-pal-log");
    if (!log) return;
    const div = document.createElement("div");
    div.className = "np-msg np-" + role;
    div.textContent = text;
    log.appendChild(div);
    log.scrollTop = log.scrollHeight;
  }

  function typeInto(el, text) {
    clearInterval(el._timer);
    el.textContent = "";
    let i = 0;
    el._timer = setInterval(() => {
      if (i >= text.length) { clearInterval(el._timer); return; }
      el.textContent += text[i];
      i += 1;
    }, 10);
  }

  async function ask(question) {
    if (!dataset.length) { setStatus("Dataset not loaded"); return; }
    bubble("user", question);
    setStatus("The board is deliberating...");
    document.dispatchEvent(new CustomEvent("neon-pal:thinking"));
    await new Promise((r) => setTimeout(r, 900));
    const match = findBest(question);
    Object.keys(SLOTS).forEach((seat, i) => {
      const el = $(SLOTS[seat]);
      if (el) setTimeout(() => typeInto(el, match.takes[seat] || ""), i * 400);
    });
    bubble("bot", "Neon Pal: " + match.takes.recommendation);
    setStatus("Board recommendation ready");
    document.dispatchEvent(new CustomEvent("neon-pal:answered", { detail: match }));
  }

  document.addEventListener("DOMContentLoaded", async () => {
    const form = $("#neon-pal-form");
    const input = $("#neon-pal-input");
    if (form && input) {
      form.addEventListener("submit", (e) => {
        e.preventDefault();
        const q = input.value.trim();
        if (!q) return;
        input.value = "";
        ask(q);
      });
    }
    try {
      await loadDataset();
    } catch (err) {
      setStatus("Dataset not found in public/");
      console.error(err);
    }
  });

  window.NeonPal = { ask };
})();
'''

AVATAR_JS = r'''// neon-pal-avatar.js - renders the Mixamo GLB and reacts to chat events.
import * as THREE from "three";
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";

const GLB_FILE = "__GLB__"; // file name inside public/

const stage = document.getElementById("neon-pal-stage");
const scene = new THREE.Scene();
const camera = new THREE.PerspectiveCamera(35, stage.clientWidth / stage.clientHeight, 0.1, 100);
const renderer = new THREE.WebGLRenderer({ antialias: true, alpha: true });
renderer.setPixelRatio(window.devicePixelRatio);
renderer.setSize(stage.clientWidth, stage.clientHeight);
stage.appendChild(renderer.domElement);

scene.add(new THREE.HemisphereLight(0xffffff, 0x223355, 1.4));
const key = new THREE.DirectionalLight(0xffffff, 1.6);
key.position.set(2, 4, 3);
scene.add(key);
const rim = new THREE.DirectionalLight(0x00d4ff, 2.2);
rim.position.set(-3, 2, -2);
scene.add(rim);
const fill = new THREE.DirectionalLight(0x0070ba, 1.2);
fill.position.set(3, 1, 1);
scene.add(fill);

let mixer = null;
let current = null;
let model = null;
let mood = "idle";
let baseY = 0;
let height = 1;
let phase = 0;
let turn = 0;
let bob = 0;
let thinkWeight = 0;
let talkWeight = 0;
let time = 0;
const rig = {}; // head and arm bones with their rest orientation, filled in after load
const actions = {};
const clock = new THREE.Clock();

// Clip-name keywords tried in order against the clips in your GLB. When no clip matches a state,
// the avatar holds its rest pose and moves procedurally (see MOTION), so a GLB without idle,
// thinking, or talking clips still looks alive.
const STATES = {
  idle: ["idle", "breath", "stand"],
  thinking: ["think", "look"],
  talking: ["talk", "gesture", "wave"],
};
// Procedural motion per state: how far it turns (radians), how fast, and how much it bobs
// (as a fraction of its height).
const MOTION = {
  idle: { turn: 0.22, speed: 1.0, bob: 0.008 },
  thinking: { turn: 0.55, speed: 2.4, bob: 0.004 },
  talking: { turn: 0.1, speed: 5.0, bob: 0.018 },
};
const ARMS_DOWN = 1.1; // radians the arms are lowered from the T-pose; 0 keeps the T-pose

function findAction(keys) {
  const names = Object.keys(actions);
  for (const k of keys) {
    const hit = names.find((n) => n.toLowerCase().includes(k));
    if (hit) return actions[hit];
  }
  return null;
}

function play(state) {
  mood = STATES[state] ? state : "idle";
  if (!mixer) return;
  const next = findAction(STATES[mood]);
  if (next === current) return;
  if (next) next.reset().fadeIn(0.3).play();
  if (current) current.fadeOut(0.3);
  current = next;
}

function findBone(pattern) {
  let bone = null;
  model.traverse((o) => { if (!bone && o.isBone && pattern.test(o.name)) bone = o; });
  return bone;
}

// Remembers a bone's rest orientation so poseBone can turn it around the model's own axes
// (x = side to side, y = up, z = front), whatever the bone's local orientation is.
function captureBone(key, pattern) {
  const bone = findBone(pattern);
  if (!bone) return;
  rig[key] = {
    bone,
    rest: bone.quaternion.clone(),
    parent: bone.parent.getWorldQuaternion(new THREE.Quaternion()),
  };
}

const _euler = new THREE.Euler();
const _delta = new THREE.Quaternion();
const _inverse = new THREE.Quaternion();
function poseBone(key, pitch, yaw, roll) {
  const part = rig[key];
  if (!part) return;
  _delta.setFromEuler(_euler.set(pitch, yaw, roll));
  _inverse.copy(part.parent).invert();
  part.bone.quaternion.copy(_inverse).multiply(_delta).multiply(part.parent).multiply(part.rest);
}

// Rotates a bone around the world front-to-back axis, whatever the bone's own orientation.
function lowerArm(pattern, angle) {
  let bone = null;
  model.traverse((o) => { if (!bone && o.isBone && pattern.test(o.name)) bone = o; });
  if (!bone || !angle) return;
  const spin = new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(0, 0, 1), angle);
  const parentWorld = bone.parent.getWorldQuaternion(new THREE.Quaternion());
  const boneWorld = bone.getWorldQuaternion(new THREE.Quaternion());
  bone.quaternion.copy(parentWorld.invert().multiply(spin.multiply(boneWorld)));
}

new GLTFLoader().load(
  encodeURI(GLB_FILE),
  (gltf) => {
    model = gltf.scene;
    scene.add(model);

    mixer = new THREE.AnimationMixer(model);
    gltf.animations.forEach((clip) => { actions[clip.name] = mixer.clipAction(clip); });
    console.log("Neon Pal clips:", Object.keys(actions));
    play("idle");
    mixer.update(0);

    model.updateMatrixWorld(true);
    if (!current) {
      lowerArm(/LeftArm$/, -ARMS_DOWN);
      lowerArm(/RightArm$/, ARMS_DOWN);
      model.updateMatrixWorld(true);
    }
    captureBone("head", /Head$/);
    captureBone("leftArm", /LeftArm$/);
    captureBone("rightArm", /RightArm$/);

    // Frame the camera on the posed model: its skeleton plus its skinned mesh.
    const box = new THREE.Box3();
    const point = new THREE.Vector3();
    model.traverse((o) => {
      if (o.isBone) box.expandByPoint(point.setFromMatrixPosition(o.matrixWorld));
      if (o.isSkinnedMesh) {
        o.frustumCulled = false;
        o.skeleton.update();
        o.computeBoundingBox();
        box.union(o.boundingBox.clone().applyMatrix4(o.matrixWorld));
      }
    });
    if (box.isEmpty()) box.setFromObject(model);
    const size = box.getSize(new THREE.Vector3());
    const center = box.getCenter(new THREE.Vector3());
    height = size.y;
    baseY = model.position.y;
    const span = Math.max(size.y, size.x / camera.aspect);
    const dist = (span / 2 / Math.tan((camera.fov * Math.PI) / 360)) * 1.15 + size.z / 2;
    camera.position.set(center.x, center.y, center.z + dist);
    camera.near = dist / 100;
    camera.far = dist * 100;
    camera.updateProjectionMatrix();
    camera.lookAt(center);
  },
  undefined,
  (err) => console.error("Could not load " + GLB_FILE, err)
);

document.addEventListener("neon-pal:thinking", () => play("thinking"));
document.addEventListener("neon-pal:answered", () => {
  play("talking");
  setTimeout(() => play("idle"), 6000);
});

function animate() {
  requestAnimationFrame(animate);
  const dt = clock.getDelta();
  if (mixer) mixer.update(dt);
  if (model) {
    // Ease toward the current state's motion; a playing clip replaces the procedural motion.
    const target = current ? { turn: 0, speed: 1, bob: 0 } : MOTION[mood];
    const ease = Math.min(1, dt * 4);
    phase += dt * target.speed;
    turn += (target.turn - turn) * ease;
    bob += (target.bob - bob) * ease;
    model.rotation.y = Math.sin(phase * 0.6) * turn;
    model.position.y = baseY + Math.sin(phase * 1.7) * bob * height;

    // Thinking: head tilts and looks up, drifting slowly. Talking: head nods and the arms gesture.
    time += dt;
    thinkWeight += ((!current && mood === "thinking" ? 1 : 0) - thinkWeight) * ease;
    talkWeight += ((!current && mood === "talking" ? 1 : 0) - talkWeight) * ease;
    if (!current) {
      const k = thinkWeight;
      const t = talkWeight;
      poseBone("head",
        k * (-0.2 + Math.sin(time * 1.3) * 0.04) + t * Math.sin(time * 7.5) * 0.09,
        k * Math.sin(time * 0.9) * 0.18 + t * Math.sin(time * 2.6) * 0.14,
        k * 0.24 + t * Math.sin(time * 3.1) * 0.05);
      poseBone("rightArm", t * (-0.15 + Math.sin(time * 4.2) * 0.2), t * Math.sin(time * 2.1) * 0.15, 0);
      poseBone("leftArm", t * Math.sin(time * 4.2 + 1.4) * 0.1, 0, t * (0.12 + Math.sin(time * 3.4) * 0.1));
    }
  }
  renderer.render(scene, camera);
}
animate();

window.addEventListener("resize", () => {
  camera.aspect = stage.clientWidth / stage.clientHeight;
  camera.updateProjectionMatrix();
  renderer.setSize(stage.clientWidth, stage.clientHeight);
});

window.NeonPalAvatar = { play };
'''

glbs = sorted(glob.glob("public/*.glb"))
glb_name = os.path.basename(glbs[0]) if glbs else "PUT_YOUR_FILE.glb"

files = {
    "dataset/README.md": README,
    "dataset/push_dataset.py": PUSH,
    "dataset/pull_dataset.py": PULL,
    "public/boardroom.html": HTML,
    "public/boardroom-layout.css": CSS,
    "public/neon-pal-chat.js": CHAT_JS,
    "public/neon-pal-avatar.js": AVATAR_JS.replace("__GLB__", glb_name),
}
for path, content in files.items():
    with open(path, "w") as f:
        f.write(content)
    print("wrote", path)

print(f"wrote {len(rows)} examples to dataset/ and public/")
if glbs:
    print("Avatar will load:", glb_name)
else:
    print("No .glb found in public/. Copy your GLB there and set GLB_FILE in public/neon-pal-avatar.js")
