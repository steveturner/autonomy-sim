import './style.css';
import milsymbol from 'milsymbol';
import type {
  EntityEffectState,
  EntityState,
  FireCellState,
  HelloEnvelope,
  LinkEvent,
  LinkState,
  LinkType,
  ScenarioSummary,
  StateEnvelope,
  TrafficState,
  BaseState,
} from './types';

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

const entityVisuals = new Map<string, {
  marker: any;
  trail: any;
  history: number[];
  symbolKey: string;
}>();
const effectVisuals = new Map<string, { entity: any; visualKind: 'area' | 'ring' | 'line' }>();
const svgEffectVisuals = new Map<string, {
  element: SVGCircleElement | SVGLineElement;
  spec: EffectVisualSpec;
}>();
const linkVisuals = new Map<string, any>();
const svgLinkVisuals = new Map<string, {
  line: SVGLineElement; source: string; target: string; linkType: LinkType;
}>();
const entityPositions = new Map<string, EntityState['position']>();
const symbolImages = new Map<string, { image: HTMLCanvasElement; width: number; height: number }>();
let hasFramed = false;
let googleTiles: any = null;
let reconnectTimer = 0;
let socket: WebSocket | null = null;
let connectionGeneration = 0;
let lastSequence = -1;
let selectedScenario: ScenarioSummary | null = null;
let scenarios: ScenarioSummary[] = [];

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

function normalizedAffiliation(entity: EntityState): 'friendly' | 'hostile' | 'neutral' | 'unknown' {
  const value = String(entity.affiliation || '').toLowerCase();
  if (value === 'friendly' || value === 'friend' || value === 'f') return 'friendly';
  if (value === 'hostile' || value === 'h') return 'hostile';
  if (value === 'neutral' || value === 'n') return 'neutral';
  if (entity.kind === 'threat_uas') return 'hostile';
  return entity.sidc ? 'unknown' : 'friendly';
}

function affiliationColor(entity: EntityState): any {
  const colors: Record<ReturnType<typeof normalizedAffiliation>, string> = {
    friendly: '#38bdf8',
    hostile: '#fb4f5f',
    neutral: '#4ade80',
    unknown: '#facc15',
  };
  return Cesium.Color.fromCssColorString(colors[normalizedAffiliation(entity)]);
}

function fallbackSidc(entity: EntityState): string {
  const affiliationCode: Record<ReturnType<typeof normalizedAffiliation>, string> = {
    friendly: 'F', hostile: 'H', neutral: 'N', unknown: 'U',
  };
  const affiliation = affiliationCode[normalizedAffiliation(entity)];
  if (entity.domain === 'air' || entity.kind === 'drone' || entity.kind === 'threat_uas') {
    return `S${affiliation}APMFQ--------`;
  }
  if (entity.kind === 'person') return `S${affiliation}GPUCI--------`;
  return `S${affiliation}GPU----------`;
}

function symbolFor(entity: EntityState): {
  key: string;
  image: HTMLCanvasElement;
  width: number;
  height: number;
} {
  const sidc = entity.sidc?.trim() || fallbackSidc(entity);
  const key = sidc.toUpperCase();
  let cached = symbolImages.get(key);
  if (!cached) {
    try {
      const image = new milsymbol.Symbol(sidc, {
        size: 36,
        padding: 2,
        infoFields: false,
        standard: '2525',
      }).asCanvas(2);
      const maxDimension = Math.max(image.width, image.height, 1);
      cached = {
        image,
        width: Math.max(24, Math.round((image.width / maxDimension) * 50)),
        height: Math.max(24, Math.round((image.height / maxDimension) * 50)),
      };
    } catch (error) {
      console.warn(`Unable to render SIDC ${sidc}; using a generic symbol`, error);
      const image = new milsymbol.Symbol('SUGPU----------', { size: 36, padding: 2 }).asCanvas(2);
      cached = { image, width: 42, height: 42 };
    }
    symbolImages.set(key, cached);
  }
  return { key, ...cached };
}

function compactLabel(entity: EntityState): string {
  const callsign = entity.callsign?.trim() || entity.name;
  const missionState = entity.mission_state?.trim()
    || entity.mission.active_node
    || entity.mission.status;
  return `${callsign} · ${missionState}`;
}

function updateEntities(entities: EntityState[]): void {
  const current = new Set<string>();
  for (const entity of entities) {
    current.add(entity.id);
    entityPositions.set(entity.id, entity.position);
    const cartesian = Cesium.Cartesian3.fromDegrees(entity.position.lon_deg, entity.position.lat_deg, entity.position.alt_m);
    const symbol = symbolFor(entity);
    const heading = -Cesium.Math.toRadians(entity.heading_deg ?? entity.kinematics.heading_deg);
    const existing = entityVisuals.get(entity.id);
    if (existing) {
      existing.marker.position = cartesian;
      existing.marker.billboard.rotation = heading;
      existing.marker.label.text = compactLabel(entity);
      existing.marker.description = description(entity);
      if (existing.symbolKey !== symbol.key) {
        existing.marker.billboard.image = symbol.image;
        existing.marker.billboard.width = symbol.width;
        existing.marker.billboard.height = symbol.height;
        existing.symbolKey = symbol.key;
      }
      existing.history.push(entity.position.lon_deg, entity.position.lat_deg, entity.position.alt_m);
      if (existing.history.length > 270) existing.history.splice(0, 3);
      existing.trail.polyline.positions = Cesium.Cartesian3.fromDegreesArrayHeights(existing.history);
      continue;
    }
    const color = affiliationColor(entity);
    const marker = viewer.entities.add({
      id: `entity/${entity.id}`,
      name: entity.name,
      position: cartesian,
      billboard: {
        image: symbol.image,
        width: symbol.width,
        height: symbol.height,
        rotation: heading,
        alignedAxis: Cesium.Cartesian3.ZERO,
        disableDepthTestDistance: Number.POSITIVE_INFINITY,
      },
      label: {
        text: compactLabel(entity),
        font: '600 12px IBM Plex Mono, monospace',
        fillColor: Cesium.Color.WHITE,
        outlineColor: Cesium.Color.BLACK,
        outlineWidth: 3,
        style: Cesium.LabelStyle.FILL_AND_OUTLINE,
        pixelOffset: new Cesium.Cartesian2(0, -34),
        disableDepthTestDistance: Number.POSITIVE_INFINITY,
      },
      description: description(entity),
    });
    const history = [entity.position.lon_deg, entity.position.lat_deg, entity.position.alt_m];
    const trail = viewer.entities.add({
      id: `track/${entity.id}`,
      polyline: { positions: [cartesian], width: 1.5, material: color.withAlpha(0.48) },
    });
    entityVisuals.set(entity.id, { marker, trail, history, symbolKey: symbol.key });
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
    <tr><th>Callsign</th><td>${escapeHtml(entity.callsign || entity.name)}</td></tr>
    <tr><th>SIDC</th><td>${escapeHtml(entity.sidc || 'legacy fallback')}</td></tr>
    <tr><th>Affiliation</th><td>${escapeHtml(normalizedAffiliation(entity))}</td></tr>
    <tr><th>Type</th><td>${escapeHtml(entity.kind)} / ${escapeHtml(entity.domain)}</td></tr>
    <tr><th>Mission</th><td>${escapeHtml(entity.mission.playbook)}</td></tr>
    ${entity.mission_role ? `<tr><th>Role</th><td>${escapeHtml(entity.mission_role)}</td></tr>` : ''}
    <tr><th>Active node</th><td>${escapeHtml(entity.mission_state || entity.mission.active_node)}</td></tr>
    ${entity.retardant_pct !== undefined ? `<tr><th>Retardant</th><td>${entity.retardant_pct.toFixed(0)}%</td></tr>` : ''}
    <tr><th>Speed</th><td>${entity.kinematics.speed_mps.toFixed(1)} m/s</td></tr>
    <tr><th>Altitude</th><td>${entity.position.alt_m.toFixed(0)} m</td></tr>
  </tbody></table>`;
}

function escapeHtml(value: unknown): string {
  return String(value)
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#039;');
}

interface EffectVisualSpec {
  id: string;
  visualKind: 'area' | 'ring' | 'line';
  name: string;
  position: EntityState['position'];
  target?: EntityState['position'];
  radiusM: number;
  pointSize?: number;
  color: any;
}

function numberValue(...values: unknown[]): number | undefined {
  for (const value of values) {
    if (typeof value === 'number' && Number.isFinite(value)) return value;
  }
  return undefined;
}

function normalizedIntensity(value: unknown): number {
  const numeric = numberValue(value) ?? 0.5;
  return Cesium.Math.clamp(numeric > 1 ? numeric / 100 : numeric, 0, 1);
}

function effectMatches(effect: EntityEffectState, terms: string[]): boolean {
  const kind = effect.kind.toLowerCase();
  return terms.some((term) => kind.includes(term));
}

function isActive(entity: EntityState, effect?: EntityEffectState): boolean {
  if (effect?.active !== undefined) return effect.active;
  if (entity.active !== undefined) return entity.active;
  return entity.mission.status === 'running';
}

function collectEffectSpecs(
  entities: EntityState[],
  wireEffects: EntityEffectState[],
  fireCells: FireCellState[],
  base: BaseState | null,
): EffectVisualSpec[] {
  const specs: EffectVisualSpec[] = [];
  const byEntityId = new Map(entities.map((entity) => [entity.id, entity]));
  const positionFor = (id: string | undefined, fallback?: EntityState['position']) => (
    (id && byEntityId.get(id)?.position) || fallback
  );

  for (const cell of fireCells) {
    const intensity = normalizedIntensity(cell.intensity);
    specs.push({
      id: `effect/fire-cell/${cell.id}`,
      visualKind: 'area',
      name: `FIRE CELL / ${cell.id} · ${cell.status}${cell.assigned_tanker ? ` · ${cell.assigned_tanker}` : ''}`,
      position: cell.position,
      radiusM: 70 + intensity * 280,
      pointSize: 16 + intensity * 24,
      color: Cesium.Color.lerp(
        Cesium.Color.fromCssColorString('#fbbf24'),
        Cesium.Color.fromCssColorString('#ef233c'),
        intensity,
        new Cesium.Color(),
      ),
    });
  }

  if (base && !byEntityId.has(base.id)) {
    specs.push({
      id: `effect/base/${base.id}`,
      visualKind: 'ring',
      name: `BASE / ${base.name} · ${base.occupied_slots.length}/${base.reload_slots} SLOTS`,
      position: base.position,
      radiusM: 85,
      color: Cesium.Color.fromCssColorString('#38bdf8'),
    });
  }

  for (const entity of entities) {
    const nestedEffects = entity.effects || [];
    if (entity.kind === 'fire') {
      const intensity = normalizedIntensity(entity.intensity);
      const color = Cesium.Color.lerp(
        Cesium.Color.fromCssColorString('#fbbf24'),
        Cesium.Color.fromCssColorString('#ef233c'),
        intensity,
        new Cesium.Color(),
      );
      specs.push({
        id: `effect/entity/${entity.id}/fire`,
        visualKind: 'area',
        name: `FIRE CELL / ${entity.name}`,
        position: entity.position,
        radiusM: numberValue(entity.effect_radius_m, entity.radius_m) ?? 70 + intensity * 280,
        pointSize: 16 + intensity * 24,
        color,
      });
    }

    if (entity.kind === 'base' || entity.kind === 'protected_site') {
      specs.push({
        id: `effect/entity/${entity.id}/site`,
        visualKind: 'ring',
        name: `${entity.kind.toUpperCase()} / ${entity.name}`,
        position: entity.position,
        radiusM: numberValue(entity.effect_radius_m, entity.radius_m) ?? 85,
        color: affiliationColor(entity),
      });
    }

    const jammerEffect = nestedEffects.find((effect) => effectMatches(effect, ['jam', 'electronic_warfare', 'ew_']));
    if ((entity.kind === 'ew_jammer' || entity.kind === 'jammer' || jammerEffect) && isActive(entity, jammerEffect)) {
      specs.push({
        id: `effect/entity/${entity.id}/jammer`,
        visualKind: 'ring',
        name: `EW JAMMING / ${entity.name}`,
        position: entity.position,
        radiusM: numberValue(jammerEffect?.radius_m, entity.effect_radius_m, entity.radius_m) ?? 850,
        color: Cesium.Color.fromCssColorString('#c084fc'),
      });
    }

    const engagement = nestedEffects.find((effect) => effectMatches(effect, ['engage', 'intercept', 'gun']));
    const targetId = engagement?.target_entity_id || entity.engagement_target_id || entity.target_id;
    if ((entity.kind === 'interceptor' || entity.kind === 'gun_system' || engagement) && isActive(entity, engagement)) {
      const target = positionFor(targetId);
      specs.push({
        id: `effect/entity/${entity.id}/engagement`,
        visualKind: target ? 'line' : 'ring',
        name: `ENGAGEMENT / ${entity.name}${targetId ? ` → ${targetId}` : ''}`,
        position: entity.position,
        target,
        radiusM: numberValue(engagement?.radius_m, entity.effect_radius_m, entity.radius_m) ?? 180,
        color: Cesium.Color.fromCssColorString(entity.kind === 'gun_system' ? '#fb923c' : '#f43f5e'),
      });
    }
  }

  wireEffects.forEach((effect, index) => {
    if (effect.active === false) return;
    const position = positionFor(effect.source_entity_id, effect.position);
    if (!position) return;
    const key = effect.id || `${effect.kind}/${effect.source_entity_id || index}`;
    if (effectMatches(effect, ['fire'])) {
      const intensity = normalizedIntensity(effect.intensity);
      specs.push({
        id: `effect/wire/${key}`,
        visualKind: 'area',
        name: effect.kind.toUpperCase(),
        position,
        radiusM: effect.radius_m ?? 70 + intensity * 280,
        pointSize: 16 + intensity * 24,
        color: Cesium.Color.lerp(Cesium.Color.YELLOW, Cesium.Color.RED, intensity, new Cesium.Color()),
      });
    } else if (effectMatches(effect, ['jam', 'electronic_warfare', 'ew_'])) {
      specs.push({
        id: `effect/wire/${key}`,
        visualKind: 'ring',
        name: effect.kind.toUpperCase(),
        position,
        radiusM: effect.radius_m ?? 850,
        color: Cesium.Color.fromCssColorString('#c084fc'),
      });
    } else if (effectMatches(effect, ['engage', 'intercept', 'gun'])) {
      const target = positionFor(effect.target_entity_id);
      specs.push({
        id: `effect/wire/${key}`,
        visualKind: target ? 'line' : 'ring',
        name: effect.kind.toUpperCase(),
        position,
        target,
        radiusM: effect.radius_m ?? 180,
        color: Cesium.Color.fromCssColorString('#fb923c'),
      });
    }
  });

  return specs;
}

function updateEffects(
  entities: EntityState[],
  wireEffects: EntityEffectState[] = [],
  fireCells: FireCellState[] = [],
  base: BaseState | null = null,
): void {
  const active = new Set<string>();
  for (const spec of collectEffectSpecs(entities, wireEffects, fireCells, base)) {
    active.add(spec.id);
    updateEffectOverlay(spec);
    const position = Cesium.Cartesian3.fromDegrees(
      spec.position.lon_deg,
      spec.position.lat_deg,
      spec.position.alt_m,
    );
    const existing = effectVisuals.get(spec.id);
    if (existing && existing.visualKind === spec.visualKind) {
      existing.entity.position = position;
      existing.entity.name = spec.name;
      if (spec.visualKind === 'line' && spec.target) {
        existing.entity.polyline.positions = Cesium.Cartesian3.fromDegreesArrayHeights([
          spec.position.lon_deg, spec.position.lat_deg, spec.position.alt_m,
          spec.target.lon_deg, spec.target.lat_deg, spec.target.alt_m,
        ]);
        existing.entity.polyline.material = new Cesium.PolylineDashMaterialProperty({ color: spec.color });
      } else {
        existing.entity.ellipse.semiMajorAxis = spec.radiusM;
        existing.entity.ellipse.semiMinorAxis = spec.radiusM;
        existing.entity.ellipse.material = spec.color.withAlpha(spec.visualKind === 'area' ? 0.32 : 0.06);
        existing.entity.ellipse.outlineColor = spec.color.withAlpha(0.9);
        if (existing.entity.point && spec.pointSize) {
          existing.entity.point.pixelSize = spec.pointSize;
          existing.entity.point.color = spec.color.withAlpha(0.9);
        }
      }
      continue;
    }
    if (existing) viewer.entities.remove(existing.entity);

    const visual = spec.visualKind === 'line' && spec.target
      ? viewer.entities.add({
        id: spec.id,
        name: spec.name,
        polyline: {
          positions: Cesium.Cartesian3.fromDegreesArrayHeights([
            spec.position.lon_deg, spec.position.lat_deg, spec.position.alt_m,
            spec.target.lon_deg, spec.target.lat_deg, spec.target.alt_m,
          ]),
          width: 3,
          material: new Cesium.PolylineDashMaterialProperty({ color: spec.color, dashLength: 10 }),
          arcType: Cesium.ArcType.NONE,
        },
      })
      : viewer.entities.add({
        id: spec.id,
        name: spec.name,
        position,
        ellipse: {
          semiMajorAxis: spec.radiusM,
          semiMinorAxis: spec.radiusM,
          material: spec.color.withAlpha(spec.visualKind === 'area' ? 0.32 : 0.06),
          outline: true,
          outlineColor: spec.color.withAlpha(0.9),
          height: Math.max(0, spec.position.alt_m) + 2,
        },
        point: spec.visualKind === 'area' ? {
          pixelSize: spec.pointSize || 18,
          color: spec.color.withAlpha(0.9),
          outlineColor: Cesium.Color.WHITE.withAlpha(0.85),
          outlineWidth: 1.5,
          disableDepthTestDistance: Number.POSITIVE_INFINITY,
        } : undefined,
      });
    effectVisuals.set(spec.id, { entity: visual, visualKind: spec.visualKind });
  }

  for (const [id, visual] of effectVisuals) {
    if (!active.has(id)) {
      viewer.entities.remove(visual.entity);
      effectVisuals.delete(id);
    }
  }
  for (const [id, visual] of svgEffectVisuals) {
    if (!active.has(id)) {
      visual.element.remove();
      svgEffectVisuals.delete(id);
    }
  }
}

function updateEffectOverlay(spec: EffectVisualSpec): void {
  const existing = svgEffectVisuals.get(spec.id);
  const needsLine = spec.visualKind === 'line';
  if (existing && (existing.element instanceof SVGLineElement) !== needsLine) {
    existing.element.remove();
    svgEffectVisuals.delete(spec.id);
  }
  const current = svgEffectVisuals.get(spec.id);
  const element = current?.element || document.createElementNS(
    'http://www.w3.org/2000/svg',
    needsLine ? 'line' : 'circle',
  );
  const cssColor = spec.color.toCssColorString();
  element.style.color = cssColor;
  element.setAttribute('stroke', cssColor);
  if (needsLine) {
    element.setAttribute('class', 'effect-line');
  } else {
    element.setAttribute('class', spec.visualKind === 'area' ? 'effect-area' : 'effect-ring');
    element.setAttribute('fill', spec.visualKind === 'area' ? cssColor : 'none');
    element.setAttribute('fill-opacity', spec.visualKind === 'area' ? '0.22' : '0');
  }
  if (!current) byId<SVGSVGElement>('effectOverlay').append(element);
  svgEffectVisuals.set(spec.id, { element, spec });
}

function screenPosition(position: EntityState['position']): any {
  return Cesium.SceneTransforms.worldToWindowCoordinates(
    viewer.scene,
    Cesium.Cartesian3.fromDegrees(position.lon_deg, position.lat_deg, position.alt_m),
  );
}

function syncEffectOverlay(): void {
  for (const visual of svgEffectVisuals.values()) {
    const source = screenPosition(visual.spec.position);
    if (!source) {
      visual.element.style.display = 'none';
      continue;
    }
    if (visual.element instanceof SVGLineElement) {
      const target = visual.spec.target && screenPosition(visual.spec.target);
      if (!target) {
        visual.element.style.display = 'none';
        continue;
      }
      visual.element.setAttribute('x1', String(source.x));
      visual.element.setAttribute('y1', String(source.y));
      visual.element.setAttribute('x2', String(target.x));
      visual.element.setAttribute('y2', String(target.y));
    } else {
      const metersPerLongitudeDegree = Math.max(
        1,
        111_320 * Math.cos(Cesium.Math.toRadians(visual.spec.position.lat_deg)),
      );
      const edge = screenPosition({
        ...visual.spec.position,
        lon_deg: visual.spec.position.lon_deg + visual.spec.radiusM / metersPerLongitudeDegree,
      });
      if (!edge) {
        visual.element.style.display = 'none';
        continue;
      }
      visual.element.setAttribute('cx', String(source.x));
      visual.element.setAttribute('cy', String(source.y));
      visual.element.setAttribute('r', String(Math.max(5, Math.hypot(edge.x - source.x, edge.y - source.y))));
    }
    visual.element.style.display = '';
  }
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

function frameScenario(positions: EntityState['position'][]): void {
  const longitudes = positions.map((position) => position.lon_deg);
  const latitudes = positions.map((position) => position.lat_deg);
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

function clearDynamicVisuals(): void {
  for (const visual of entityVisuals.values()) {
    viewer.entities.remove(visual.marker);
    viewer.entities.remove(visual.trail);
  }
  for (const visual of effectVisuals.values()) viewer.entities.remove(visual.entity);
  for (const visual of linkVisuals.values()) viewer.entities.remove(visual);
  for (const visual of svgLinkVisuals.values()) visual.line.remove();
  entityVisuals.clear();
  effectVisuals.clear();
  for (const visual of svgEffectVisuals.values()) visual.element.remove();
  svgEffectVisuals.clear();
  linkVisuals.clear();
  svgLinkVisuals.clear();
  entityPositions.clear();
  byId<HTMLOListElement>('eventLog').innerHTML = '<li class="muted">No transitions yet</li>';
  hasFramed = false;
  lastSequence = -1;
}

function setScenarioHeading(scenario: string): void {
  byId('scenarioName').textContent = scenario.toUpperCase();
  const matching = scenarios.find((item) => item.id === scenario || item.name === scenario);
  if (matching) {
    selectedScenario = matching;
    byId<HTMLSelectElement>('scenarioSelect').value = matching.id;
  }
}

function handleMessage(value: HelloEnvelope | StateEnvelope): void {
  if (value.schema !== 'autonomy-sim/v1') return;
  if (value.message_type === 'hello') {
    setScenarioHeading(value.payload.scenario);
    return;
  }
  if (value.sequence <= lastSequence) return;
  lastSequence = value.sequence;
  updateEntities(value.payload.entities);
  updateEffects(
    value.payload.entities,
    value.payload.effects,
    value.payload.fire_cells,
    value.payload.base,
  );
  updateLinks(value.payload.links, value.payload.traffic);
  updateHud(value);
  if (!hasFramed && value.payload.entities.length) {
    hasFramed = true;
    frameScenario([
      ...value.payload.entities.map((entity) => entity.position),
      ...(value.payload.fire_cells || []).map((cell) => cell.position),
      ...(value.payload.base ? [value.payload.base.position] : []),
    ]);
  }
}

function connect(): void {
  const generation = ++connectionGeneration;
  window.clearTimeout(reconnectTimer);
  if (socket) {
    socket.onclose = null;
    socket.close();
  }
  const url = streamUrl(selectedScenario);
  const indicator = byId('connection');
  indicator.className = 'connection pending';
  indicator.innerHTML = '<span></span>CONNECTING';
  lastSequence = -1;
  socket = new WebSocket(url);
  socket.onopen = () => {
    if (generation !== connectionGeneration) return;
    indicator.className = 'connection online';
    indicator.innerHTML = '<span></span>LIVE';
  };
  socket.onmessage = (event) => {
    if (generation !== connectionGeneration) return;
    try { handleMessage(JSON.parse(event.data)); }
    catch (error) { console.error('Invalid state message', error); }
  };
  socket.onclose = () => {
    if (generation !== connectionGeneration) return;
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

function streamUrl(scenario: ScenarioSummary | null = selectedScenario): string {
  const configuredWebSocket = import.meta.env.VITE_WS_URL as string | undefined;
  const endpoint = scenario?.stream_url || configuredWebSocket;
  const url = endpoint ? new URL(endpoint, apiBaseUrl()) : apiBaseUrl();
  url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
  if (!endpoint) {
    url.pathname = '/api/v1/stream';
    url.search = '';
  }
  if (scenario?.id) url.searchParams.set('scenario', scenario.id);
  url.hash = '';
  return url.toString();
}

function scenariosUrl(): string {
  const url = apiBaseUrl();
  url.pathname = '/api/v1/scenarios';
  url.search = '';
  url.hash = '';
  return url.toString();
}

function normalizeScenario(value: unknown): ScenarioSummary | null {
  if (typeof value === 'string' && value.trim()) {
    return { id: value, name: value };
  }
  if (!value || typeof value !== 'object') return null;
  const record = value as Record<string, unknown>;
  const id = String(record.id || record.slug || record.scenario || record.name || '').trim();
  if (!id) return null;
  return {
    id,
    name: String(record.display_name || record.title || record.name || id),
    description: typeof record.description === 'string' ? record.description : undefined,
    entity_count: typeof record.entity_count === 'number' ? record.entity_count : undefined,
    default: typeof record.default === 'boolean' ? record.default : undefined,
    builder: typeof record.builder === 'string' ? record.builder : undefined,
    stream_url: typeof record.stream_url === 'string'
      ? record.stream_url
      : typeof record.ws_url === 'string' ? record.ws_url : undefined,
  };
}

async function loadScenarios(): Promise<void> {
  const select = byId<HTMLSelectElement>('scenarioSelect');
  try {
    const response = await fetch(scenariosUrl(), { signal: AbortSignal.timeout(5000) });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const body: unknown = await response.json();
    const record = body && typeof body === 'object' ? body as Record<string, unknown> : null;
    const activeScenarioId = typeof record?.active === 'string' ? record.active : undefined;
    const raw = Array.isArray(body)
      ? body
      : record && Array.isArray(record.scenarios)
        ? record.scenarios
        : [];
    scenarios = raw.map(normalizeScenario).filter((item): item is ScenarioSummary => Boolean(item));
    if (!scenarios.length) throw new Error('server returned no scenarios');

    select.replaceChildren(...scenarios.map((scenario) => {
      const option = document.createElement('option');
      option.value = scenario.id;
      option.textContent = scenario.name;
      if (scenario.description) option.title = scenario.description;
      return option;
    }));
    const requested = new URL(location.href).searchParams.get('scenario');
    selectedScenario = scenarios.find((scenario) => scenario.id === requested)
      || scenarios.find((scenario) => scenario.id === activeScenarioId)
      || scenarios.find((scenario) => scenario.default)
      || scenarios[0];
    select.value = selectedScenario.id;
    select.disabled = false;
    setScenarioHeading(selectedScenario.name);
  } catch (error) {
    console.warn('Scenario discovery unavailable; connecting to the server default', error);
    scenarios = [];
    selectedScenario = null;
    const option = document.createElement('option');
    option.textContent = 'SERVER DEFAULT';
    select.replaceChildren(option);
    select.disabled = true;
  }
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
byId<HTMLSelectElement>('scenarioSelect').addEventListener('change', (event) => {
  const scenarioId = (event.currentTarget as HTMLSelectElement).value;
  const nextScenario = scenarios.find((scenario) => scenario.id === scenarioId);
  if (!nextScenario || nextScenario.id === selectedScenario?.id) return;
  selectedScenario = nextScenario;
  setScenarioHeading(nextScenario.name);
  clearDynamicVisuals();
  connect();
});

void loadScenarios().finally(connect);
viewer.scene.postRender.addEventListener(syncLinkOverlay);
viewer.scene.postRender.addEventListener(syncEffectOverlay);
(window as any).autonomySim = {
  viewer,
  entityVisuals,
  effectVisuals,
  svgEffectVisuals,
  linkVisuals,
  svgLinkVisuals,
  apiUrl: apiBaseUrl().toString(),
  get streamUrl() { return streamUrl(); },
  get scenarios() { return scenarios; },
  reconnect: connect,
};
