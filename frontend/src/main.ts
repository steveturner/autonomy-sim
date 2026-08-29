import './style.css';
import type { EntityState, HelloEnvelope, LinkEvent, LinkState, LinkType, StateEnvelope, TrafficState } from './types';

declare const Cesium: any;

(window as any).CESIUM_BASE_URL = 'https://cdn.jsdelivr.net/npm/cesium@1.124/Build/Cesium/';

const cesiumIonToken = (import.meta.env.VITE_CESIUM_ION_TOKEN as string | undefined)?.trim();
if (cesiumIonToken) Cesium.Ion.defaultAccessToken = cesiumIonToken;

const viewer = new Cesium.Viewer('cesiumContainer', {
  animation: false,
  timeline: false,
  baseLayerPicker: false,
  geocoder: false,
  homeButton: false,
  sceneModePicker: false,
  navigationHelpButton: false,
  fullscreenButton: false,
  selectionIndicator: true,
  infoBox: true,
  baseLayer: false,
  sceneMode: Cesium.SceneMode.SCENE2D,
});

viewer.scene.globe.enableLighting = false;
viewer.scene.backgroundColor = Cesium.Color.fromCssColorString('#02070b');

const ESRI_WORLD_IMAGERY_URL = 'https://services.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer';

async function imageryProvider(): Promise<any> {
  const source = ((import.meta.env.VITE_IMAGERY_SOURCE as string | undefined) || 'esri').trim().toLowerCase();
  const customUrl = (import.meta.env.VITE_IMAGERY_URL as string | undefined)?.trim();

  if (source === 'url') {
    if (!customUrl) throw new Error('VITE_IMAGERY_URL is required when VITE_IMAGERY_SOURCE=url');
    return new Cesium.UrlTemplateImageryProvider({ url: customUrl });
  }
  if (source === 'esri') {
    return Cesium.ArcGisMapServerImageryProvider.fromUrl(customUrl || ESRI_WORLD_IMAGERY_URL);
  }
  if (!cesiumIonToken) {
    throw new Error(`${source} imagery requires VITE_CESIUM_ION_TOKEN`);
  }
  if (source === 'sentinel-2' || source === 'sentinel2') {
    return Cesium.IonImageryProvider.fromAssetId(3954);
  }
  if (source === 'bing-labels') {
    return Cesium.createWorldImageryAsync({ style: Cesium.IonWorldImageryStyle.AERIAL_WITH_LABELS });
  }
  if (source === 'bing' || source === 'bing-aerial' || source === 'cesium-world') {
    return Cesium.createWorldImageryAsync({ style: Cesium.IonWorldImageryStyle.AERIAL });
  }
  throw new Error(`Unsupported VITE_IMAGERY_SOURCE: ${source}`);
}

async function configureGlobe(): Promise<void> {
  try {
    viewer.imageryLayers.addImageryProvider(await imageryProvider());
  } catch (error) {
    console.warn('Configured satellite imagery unavailable; falling back to Esri World Imagery', error);
    if (((import.meta.env.VITE_IMAGERY_SOURCE as string | undefined) || 'esri').toLowerCase() !== 'esri') {
      try {
        viewer.imageryLayers.addImageryProvider(
          await Cesium.ArcGisMapServerImageryProvider.fromUrl(ESRI_WORLD_IMAGERY_URL),
        );
      } catch (fallbackError) {
        console.error('Esri World Imagery is unavailable', fallbackError);
      }
    }
  }

  if (!cesiumIonToken) return;
  try {
    viewer.terrainProvider = await Cesium.createWorldTerrainAsync({
      requestVertexNormals: true,
      requestWaterMask: true,
    });
  } catch (error) {
    console.warn('Cesium World Terrain unavailable; retaining ellipsoid terrain', error);
  }
}

void configureGlobe();

const entityVisuals = new Map<string, { marker: any; trail: any; history: number[] }>();
const linkVisuals = new Map<string, any>();
const svgLinkVisuals = new Map<string, {
  line: SVGLineElement; source: string; target: string; linkType: LinkType;
}>();
const entityPositions = new Map<string, EntityState['position']>();
let hasFramed = false;
let googleTiles: any = null;
let reconnectTimer = 0;
let socket: WebSocket | null = null;

const linkColors: Record<LinkType, any> = {
  mesh: Cesium.Color.fromCssColorString('#22d3ee'),
  cellular: Cesium.Color.fromCssColorString('#e879f9'),
  satcom: Cesium.Color.fromCssColorString('#fbbf24'),
  ble: Cesium.Color.fromCssColorString('#4ade80'),
};

const linkCssColors: Record<LinkType, string> = {
  mesh: '#22d3ee',
  cellular: '#e879f9',
  satcom: '#fbbf24',
  ble: '#4ade80',
};
const linkOffsets: Record<LinkType, number> = { mesh: -6, cellular: -2, satcom: 2, ble: 6 };

function byId<T extends Element>(id: string): T {
  const node = document.getElementById(id);
  if (!node) throw new Error(`missing element #${id}`);
  return node as unknown as T;
}

function iconFor(entity: EntityState): string {
  switch (entity.kind) {
    case 'drone': return '▲';
    case 'person': return '●';
    case 'ground_vehicle': return '◆';
    case 'ground_station': return '■';
    case 'sensor': return '◇';
  }
}

function colorFor(entity: EntityState): any {
  switch (entity.domain) {
    case 'air': return Cesium.Color.fromCssColorString('#41bfff');
    case 'ground': return Cesium.Color.fromCssColorString('#4ce69a');
    case 'maritime': return Cesium.Color.fromCssColorString('#3b82f6');
    case 'space': return Cesium.Color.fromCssColorString('#ffca3a');
  }
}

function updateEntities(entities: EntityState[]): void {
  const current = new Set<string>();
  for (const entity of entities) {
    current.add(entity.id);
    entityPositions.set(entity.id, entity.position);
    const cartesian = Cesium.Cartesian3.fromDegrees(entity.position.lon_deg, entity.position.lat_deg, entity.position.alt_m);
    const existing = entityVisuals.get(entity.id);
    if (existing) {
      existing.marker.position = cartesian;
      existing.marker.orientation = Cesium.Transforms.headingPitchRollQuaternion(
        cartesian,
        new Cesium.HeadingPitchRoll(Cesium.Math.toRadians(entity.kinematics.heading_deg), 0, 0),
      );
      existing.marker.description = description(entity);
      existing.history.push(entity.position.lon_deg, entity.position.lat_deg, entity.position.alt_m);
      if (existing.history.length > 270) existing.history.splice(0, 3);
      existing.trail.polyline.positions = Cesium.Cartesian3.fromDegreesArrayHeights(existing.history);
      continue;
    }
    const color = colorFor(entity);
    const marker = viewer.entities.add({
      id: `entity/${entity.id}`,
      name: entity.name,
      position: cartesian,
      point: {
        pixelSize: entity.kind === 'person' ? 9 : 13,
        color,
        outlineColor: Cesium.Color.WHITE.withAlpha(0.9),
        outlineWidth: 1.5,
        disableDepthTestDistance: Number.POSITIVE_INFINITY,
      },
      label: {
        text: `${iconFor(entity)}  ${entity.name}`,
        font: '600 13px IBM Plex Mono, monospace',
        fillColor: Cesium.Color.WHITE,
        outlineColor: Cesium.Color.BLACK,
        outlineWidth: 3,
        style: Cesium.LabelStyle.FILL_AND_OUTLINE,
        pixelOffset: new Cesium.Cartesian2(0, -25),
        disableDepthTestDistance: Number.POSITIVE_INFINITY,
      },
      description: description(entity),
    });
    const history = [entity.position.lon_deg, entity.position.lat_deg, entity.position.alt_m];
    const trail = viewer.entities.add({
      id: `track/${entity.id}`,
      polyline: { positions: [cartesian], width: 1.5, material: color.withAlpha(0.48) },
    });
    entityVisuals.set(entity.id, { marker, trail, history });
  }
  for (const [id, visual] of entityVisuals) {
    if (!current.has(id)) {
      viewer.entities.remove(visual.marker);
      viewer.entities.remove(visual.trail);
      entityVisuals.delete(id);
      entityPositions.delete(id);
    }
  }
}

function description(entity: EntityState): string {
  return `<table class="cesium-infoBox-defaultTable"><tbody>
    <tr><th>Type</th><td>${entity.kind} / ${entity.domain}</td></tr>
    <tr><th>Mission</th><td>${entity.mission.playbook}</td></tr>
    <tr><th>Active node</th><td>${entity.mission.active_node}</td></tr>
    <tr><th>Speed</th><td>${entity.kinematics.speed_mps.toFixed(1)} m/s</td></tr>
    <tr><th>Altitude</th><td>${entity.position.alt_m.toFixed(0)} m</td></tr>
  </tbody></table>`;
}

function updateLinks(links: LinkState[], traffic: TrafficState[]): void {
  const rates = new Map(traffic.map((item) => [item.link_id, item.tx_bps + item.rx_bps]));
  const active = new Set<string>();
  for (const link of links) {
    if (link.state !== 'up') continue;
    const source = entityPositions.get(link.source);
    const target = entityPositions.get(link.target);
    if (!source || !target) continue;
    active.add(link.id);
    const positions = Cesium.Cartesian3.fromDegreesArrayHeights([
      source.lon_deg, source.lat_deg, source.alt_m,
      target.lon_deg, target.lat_deg, target.alt_m,
    ]);
    const rateFraction = Math.min(1, (rates.get(link.id) ?? 0) / Math.max(1, link.capacity_bps));
    const color = linkColors[link.link_type].withAlpha(0.48 + 0.48 * link.quality);
    const width = 2.2 + 3.8 * rateFraction;
    const material = link.link_type === 'satcom' || link.link_type === 'ble'
      ? new Cesium.PolylineDashMaterialProperty({ color, dashLength: link.link_type === 'ble' ? 5 : 16 })
      : color;
    const existing = linkVisuals.get(link.id);
    const svgExisting = svgLinkVisuals.get(link.id);
    const svgLine = svgExisting?.line ?? document.createElementNS('http://www.w3.org/2000/svg', 'line');
    svgLine.style.color = linkCssColors[link.link_type];
    svgLine.setAttribute('stroke', linkCssColors[link.link_type]);
    svgLine.setAttribute('stroke-opacity', String(0.52 + 0.44 * link.quality));
    svgLine.setAttribute('stroke-width', String(width));
    svgLine.setAttribute(
      'stroke-dasharray',
      link.link_type === 'satcom' ? '14 9' : link.link_type === 'ble' ? '4 6' : '',
    );
    if (!svgExisting) {
      byId<SVGSVGElement>('linkOverlay').append(svgLine);
      svgLinkVisuals.set(link.id, {
        line: svgLine,
        source: link.source,
        target: link.target,
        linkType: link.link_type,
      });
    }
    if (existing) {
      existing.polyline.positions = positions;
      existing.polyline.width = width;
      existing.polyline.material = material;
    } else {
      linkVisuals.set(link.id, viewer.entities.add({
        id: link.id,
        name: `DITTO / ${link.link_type.toUpperCase()} ${link.source} ↔ ${link.target}`,
        polyline: { positions, width, material, arcType: Cesium.ArcType.GEODESIC },
        description: `Ditto peer link over ${link.link_type}<br>${link.source_peer_id} ↔ ${link.target_peer_id}<br>Quality ${(link.quality * 100).toFixed(0)}% · ${link.distance_m.toFixed(0)} m · ${link.latency_ms.toFixed(1)} ms`,
      }));
    }
  }
  for (const [id, visual] of linkVisuals) {
    if (!active.has(id)) {
      viewer.entities.remove(visual);
      linkVisuals.delete(id);
    }
  }
  for (const [id, visual] of svgLinkVisuals) {
    if (!active.has(id)) {
      visual.line.remove();
      svgLinkVisuals.delete(id);
    }
  }
}

function syncLinkOverlay(): void {
  for (const visual of svgLinkVisuals.values()) {
    const source = entityPositions.get(visual.source);
    const target = entityPositions.get(visual.target);
    if (!source || !target) continue;
    const sourceWindow = Cesium.SceneTransforms.worldToWindowCoordinates(
      viewer.scene,
      Cesium.Cartesian3.fromDegrees(source.lon_deg, source.lat_deg, source.alt_m),
    );
    const targetWindow = Cesium.SceneTransforms.worldToWindowCoordinates(
      viewer.scene,
      Cesium.Cartesian3.fromDegrees(target.lon_deg, target.lat_deg, target.alt_m),
    );
    if (!sourceWindow || !targetWindow) {
      visual.line.style.display = 'none';
      continue;
    }
    const dx = targetWindow.x - sourceWindow.x;
    const dy = targetWindow.y - sourceWindow.y;
    const length = Math.hypot(dx, dy) || 1;
    const offset = linkOffsets[visual.linkType];
    const offsetX = (-dy / length) * offset;
    const offsetY = (dx / length) * offset;
    visual.line.style.display = '';
    visual.line.setAttribute('x1', String(sourceWindow.x + offsetX));
    visual.line.setAttribute('y1', String(sourceWindow.y + offsetY));
    visual.line.setAttribute('x2', String(targetWindow.x + offsetX));
    visual.line.setAttribute('y2', String(targetWindow.y + offsetY));
  }
}

function updateHud(frame: StateEnvelope): void {
  const totalBps = frame.payload.traffic.reduce((sum, item) => sum + item.tx_bps + item.rx_bps, 0);
  byId('simTime').textContent = formatTime(frame.sim_time_s);
  byId('entityCount').textContent = String(frame.payload.entities.length);
  byId('linkCount').textContent = String(frame.payload.links.filter((link) => link.state === 'up').length);
  byId('documentCount').textContent = String(frame.payload.ditto_documents.length);
  const convergedPeers = frame.payload.ditto_peers.filter((peer) => peer.converged).length;
  byId('convergence').textContent = `${convergedPeers} / ${frame.payload.ditto_peers.length}`;
  byId('trafficRate').textContent = totalBps >= 1_000_000 ? `${(totalBps / 1_000_000).toFixed(1)} Mbps` : `${Math.round(totalBps / 1_000)} kbps`;
  for (const type of ['mesh', 'cellular', 'satcom', 'ble'] as LinkType[]) {
    byId(`${type}Count`).textContent = String(frame.payload.links.filter((link) => link.state === 'up' && link.link_type === type).length);
  }
  appendEvents(frame.payload.link_events);
}

function appendEvents(events: LinkEvent[]): void {
  const log = byId<HTMLOListElement>('eventLog');
  if (events.length && log.querySelector('.muted')) log.replaceChildren();
  for (const event of events) {
    const item = document.createElement('li');
    item.className = event.state;
    item.innerHTML = `<time>${formatTime(event.changed_at_s)}</time><strong>${event.link_type}</strong><span>${event.source} ↔ ${event.target}</span><b>${event.state}</b>`;
    log.prepend(item);
  }
  while (log.children.length > 7) log.lastElementChild?.remove();
}

function formatTime(seconds: number): string {
  const mins = Math.floor(seconds / 60).toString().padStart(2, '0');
  return `${mins}:${(seconds % 60).toFixed(1).padStart(4, '0')}`;
}

function frameScenario(entities: EntityState[]): void {
  const longitudes = entities.map((entity) => entity.position.lon_deg);
  const latitudes = entities.map((entity) => entity.position.lat_deg);
  const west = Math.min(...longitudes);
  const east = Math.max(...longitudes);
  const south = Math.min(...latitudes);
  const north = Math.max(...latitudes);
  const lonMargin = Math.max((east - west) * 0.22, 0.0025);
  const latMargin = Math.max((north - south) * 0.22, 0.0025);
  viewer.camera.setView({
    destination: Cesium.Rectangle.fromDegrees(
      west - lonMargin,
      south - latMargin,
      east + lonMargin,
      north + latMargin,
    ),
  });
}

function handleMessage(value: HelloEnvelope | StateEnvelope): void {
  if (value.schema !== 'autonomy-sim/v1') return;
  if (value.message_type === 'hello') {
    byId('scenarioName').textContent = value.payload.scenario.toUpperCase();
    return;
  }
  updateEntities(value.payload.entities);
  updateLinks(value.payload.links, value.payload.traffic);
  updateHud(value);
  if (!hasFramed && value.payload.entities.length) {
    hasFramed = true;
    frameScenario(value.payload.entities);
  }
}

function connect(): void {
  const url = streamUrl();
  const indicator = byId('connection');
  socket = new WebSocket(url);
  socket.onopen = () => {
    indicator.className = 'connection online';
    indicator.innerHTML = '<span></span>LIVE';
  };
  socket.onmessage = (event) => {
    try { handleMessage(JSON.parse(event.data)); }
    catch (error) { console.error('Invalid state message', error); }
  };
  socket.onclose = () => {
    indicator.className = 'connection offline';
    indicator.innerHTML = '<span></span>RECONNECTING';
    window.clearTimeout(reconnectTimer);
    reconnectTimer = window.setTimeout(connect, 1500);
  };
}

function apiBaseUrl(): URL {
  const configuredUrl = import.meta.env.VITE_API_URL as string | undefined;
  if (configuredUrl) return new URL(configuredUrl, location.href);

  const protocol = location.protocol === 'https:' ? 'https:' : 'http:';
  const configuredHost = import.meta.env.VITE_API_HOST as string | undefined;
  const authority = configuredHost || `${location.hostname}:9000`;
  return new URL(`${protocol}//${authority}`);
}

function streamUrl(): string {
  const configuredWebSocket = import.meta.env.VITE_WS_URL as string | undefined;
  if (configuredWebSocket) return configuredWebSocket;

  const url = apiBaseUrl();
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
  url.pathname = '/api/v1/stream';
  url.search = '';
  url.hash = '';
  return url.toString();
}

async function show3d(): Promise<void> {
  byId('mode2d').classList.remove('active');
  byId('mode3d').classList.add('active');
  viewer.scene.morphTo3D(0.7);
  const key = import.meta.env.VITE_GOOGLE_MAPS_API_KEY as string | undefined;
  if (!key || googleTiles) return;
  try {
    Cesium.GoogleMaps.defaultApiKey = key;
    googleTiles = await Cesium.createGooglePhotorealistic3DTileset({ onlyUsingWithGoogleGeocoder: true });
    viewer.scene.primitives.add(googleTiles);
    viewer.scene.globe.show = false;
  } catch (error) {
    console.warn('Photorealistic tiles unavailable; retaining the Cesium globe', error);
    viewer.scene.globe.show = true;
  }
}

function show2d(): void {
  byId('mode2d').classList.add('active');
  byId('mode3d').classList.remove('active');
  viewer.scene.globe.show = true;
  if (googleTiles) googleTiles.show = false;
  viewer.scene.morphTo2D(0.7);
}

byId<HTMLButtonElement>('mode2d').addEventListener('click', show2d);
byId<HTMLButtonElement>('mode3d').addEventListener('click', () => {
  if (googleTiles) googleTiles.show = true;
  void show3d();
});

connect();
viewer.scene.postRender.addEventListener(syncLinkOverlay);
(window as any).autonomySim = {
  viewer,
  entityVisuals,
  linkVisuals,
  svgLinkVisuals,
  apiUrl: apiBaseUrl().toString(),
  streamUrl: streamUrl(),
  reconnect: connect,
};
