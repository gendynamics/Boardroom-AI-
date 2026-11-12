// neon-pal-avatar.js - renders the Mixamo GLB and reacts to chat events.
import * as THREE from "three";
import { GLTFLoader } from "three/addons/loaders/GLTFLoader.js";

const GLB_FILE = "PayPal_Neon_Pal_Bot_20261112_animated_mixamo_All_animation.glb"; // file name inside public/

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
