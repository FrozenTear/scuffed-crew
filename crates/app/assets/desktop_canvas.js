// Desktop canvas rendering module for Wry webview.
// Injected via document::eval() on desktop targets.
// Mirrors the web_sys Canvas2D rendering from map_canvas.rs.

(function() {
    "use strict";

    const TILE_CACHE_LIMIT = 256;

    // =========================================================================
    // Tile Manager
    // =========================================================================

    class TileManager {
        constructor() {
            this.mapId = "";
            this.cache = new Map();     // key -> Image
            this.loading = new Set();   // key strings
            this.lruOrder = [];
            this.onTileLoaded = null;
            this.rafScheduled = false;
        }

        tileKey(floor, z, x, y) {
            return `${floor}/${z}/${x}/${y}`;
        }

        setMap(mapId) {
            if (this.mapId !== mapId) {
                this.mapId = mapId;
                this.cache.clear();
                this.loading.clear();
                this.lruOrder = [];
            }
        }

        clear() {
            this.cache.clear();
            this.loading.clear();
            this.lruOrder = [];
        }

        getTile(floor, z, x, y) {
            const key = this.tileKey(floor, z, x, y);
            const img = this.cache.get(key);
            if (img) {
                const idx = this.lruOrder.indexOf(key);
                if (idx !== -1) this.lruOrder.splice(idx, 1);
                this.lruOrder.push(key);
                return img;
            }
            return null;
        }

        isLoading(floor, z, x, y) {
            return this.loading.has(this.tileKey(floor, z, x, y));
        }

        loadTile(floor, z, x, y) {
            const key = this.tileKey(floor, z, x, y);
            if (this.cache.has(key) || this.loading.has(key)) return;
            this.loading.add(key);
            this.evictIfNeeded();

            const url = `/assets/maps/${this.mapId}/floors/${floor}/${z}/${x}/${y}.webp`;
            const img = new Image();
            img.onload = () => {
                this.loading.delete(key);
                this.cache.set(key, img);
                this.lruOrder.push(key);
                this.scheduleRedraw();
            };
            img.onerror = () => {
                this.loading.delete(key);
            };
            img.src = url;
        }

        evictIfNeeded() {
            while (this.cache.size >= TILE_CACHE_LIMIT && this.lruOrder.length > 0) {
                const key = this.lruOrder.shift();
                this.cache.delete(key);
            }
        }

        scheduleRedraw() {
            if (!this.rafScheduled && this.onTileLoaded) {
                this.rafScheduled = true;
                requestAnimationFrame(() => {
                    this.rafScheduled = false;
                    if (this.onTileLoaded) this.onTileLoaded();
                });
            }
        }

        zoomToLevel(canvasZoom, maxZoom) {
            const levelOffset = Math.round(-Math.log2(canvasZoom));
            return Math.max(0, Math.min(maxZoom, maxZoom - levelOffset));
        }

        visibleTiles(vx, vy, vw, vh, canvasZoom, pyramid) {
            const z = this.zoomToLevel(canvasZoom, pyramid.max_zoom);
            const tilesX = tilesAtZoom(pyramid, z)[0];
            const tilesY = tilesAtZoom(pyramid, z)[1];
            const scale = 1 << (pyramid.max_zoom - z);
            const scaledTileSize = pyramid.tile_size * scale;

            const sx = Math.floor(vx / scaledTileSize);
            const sy = Math.floor(vy / scaledTileSize);
            const ex = Math.ceil((vx + vw) / scaledTileSize);
            const ey = Math.ceil((vy + vh) / scaledTileSize);

            const tiles = [];
            for (let ty = Math.max(0, sy); ty <= Math.min(tilesY - 1, ey); ty++) {
                for (let tx = Math.max(0, sx); tx <= Math.min(tilesX - 1, ex); tx++) {
                    tiles.push([z, tx, ty]);
                }
            }
            return tiles;
        }

        tileRect(z, x, y, pyramid) {
            const scale = 1 << (pyramid.max_zoom - z);
            const s = pyramid.tile_size * scale;
            return [x * s, y * s, s, s];
        }
    }

    function tilesAtZoom(pyramid, zoom) {
        const scale = 1 << (pyramid.max_zoom - Math.min(zoom, pyramid.max_zoom));
        const w = Math.max(1, Math.ceil(pyramid.full_width / scale / pyramid.tile_size));
        const h = Math.max(1, Math.ceil(pyramid.full_height / scale / pyramid.tile_size));
        return [w, h];
    }

    // =========================================================================
    // Hero Image Cache
    // =========================================================================

    class HeroImageCache {
        constructor() {
            this.cache = new Map();
        }

        get(heroId) {
            const img = this.cache.get(heroId);
            return (img && img.complete && img.naturalWidth > 0) ? img : null;
        }

        load(heroId) {
            if (this.cache.has(heroId)) return;
            const img = new Image();
            this.cache.set(heroId, img);
            img.src = `/assets/heroes/${heroId}.png`;
        }
    }

    // =========================================================================
    // Map Fallback Image
    // =========================================================================

    let mapImage = null;
    let mapImageId = null;

    function loadMapImage(mapId, onLoad) {
        if (mapImageId === mapId && mapImage) return;
        mapImageId = mapId;
        mapImage = null;
        if (!mapId) return;
        const img = new Image();
        img.onload = () => { mapImage = img; if (onLoad) onLoad(); };
        img.onerror = () => {};
        img.src = `/assets/maps/${mapId}/main.png`;
    }

    // =========================================================================
    // Drawing Functions
    // =========================================================================

    function drawArrowhead(ctx, from, to, color) {
        const angle = Math.atan2(to.y - from.y, to.x - from.x);
        const size = 15;
        ctx.fillStyle = color;
        ctx.beginPath();
        ctx.moveTo(to.x, to.y);
        ctx.lineTo(to.x - size * Math.cos(angle - 0.5), to.y - size * Math.sin(angle - 0.5));
        ctx.lineTo(to.x - size * Math.cos(angle + 0.5), to.y - size * Math.sin(angle + 0.5));
        ctx.closePath();
        ctx.fill();
    }

    function drawHealthPacks(ctx, healthPacks, meta) {
        for (const pack of healthPacks) {
            let px = pack.x, py = pack.z;
            if (meta && meta.transform) {
                const t = meta.transform;
                px = (pack.x - t.origin_x) * t.pixels_per_meter;
                py = t.z_flip
                    ? (t.origin_z - pack.z) * t.pixels_per_meter
                    : (pack.z - t.origin_z) * t.pixels_per_meter;
            }
            const isSmall = pack.size === "small";
            const radius = isSmall ? 8 : 12;
            const fill = isSmall ? "#ffeb3b" : "#ff9800";
            const stroke = isSmall ? "#ffc107" : "#f57c00";

            ctx.shadowColor = "rgba(255, 255, 255, 0.5)";
            ctx.shadowBlur = 6;
            ctx.fillStyle = fill;
            ctx.beginPath();
            ctx.arc(px, py, radius, 0, Math.PI * 2);
            ctx.fill();
            ctx.shadowBlur = 0;

            ctx.strokeStyle = stroke;
            ctx.lineWidth = 2;
            ctx.stroke();

            const cs = radius * 0.6;
            ctx.strokeStyle = "#fff";
            ctx.lineWidth = 2.5;
            ctx.lineCap = "round";
            ctx.beginPath(); ctx.moveTo(px - cs, py); ctx.lineTo(px + cs, py); ctx.stroke();
            ctx.beginPath(); ctx.moveTo(px, py - cs); ctx.lineTo(px, py + cs); ctx.stroke();
        }
    }

    function colorToCss(c) {
        if (typeof c === "string") return c;
        return `rgb(${c.r}, ${c.g}, ${c.b})`;
    }

    function colorToCssAlpha(c, alpha) {
        if (typeof c === "string") return c;
        return `rgba(${c.r}, ${c.g}, ${c.b}, ${alpha})`;
    }

    const ICON_EMOJI = {
        skull: "\u{1F480}", warning: "⚠️", star: "⭐", flag: "\u{1F6A9}",
        eye: "\u{1F441}️", shield: "\u{1F6E1}️", target: "\u{1F3AF}", question: "❓"
    };

    function drawElement(ctx, el, fillOpacity, heroCache) {
        const color = colorToCss(el.color);
        const pos = el.position;
        const et = el.element_type;

        if (et.type === "player_marker") {
            const radius = 20;
            const x = pos.x, y = pos.y;
            let drewPortrait = false;

            if (el.hero_id) {
                const img = heroCache.get(el.hero_id);
                if (img) {
                    ctx.save();
                    ctx.beginPath();
                    ctx.arc(x, y, radius, 0, Math.PI * 2);
                    ctx.clip();
                    ctx.drawImage(img, x - radius, y - radius, radius * 2, radius * 2);
                    ctx.restore();
                    drewPortrait = true;
                } else {
                    heroCache.load(el.hero_id);
                }
            }

            if (!drewPortrait) {
                ctx.fillStyle = color;
                ctx.beginPath();
                ctx.arc(x, y, radius, 0, Math.PI * 2);
                ctx.fill();
            }

            ctx.strokeStyle = drewPortrait ? color : "#fff";
            ctx.lineWidth = drewPortrait ? 3 : 2;
            ctx.beginPath();
            ctx.arc(x, y, radius, 0, Math.PI * 2);
            ctx.stroke();

            if (!drewPortrait && el.label) {
                ctx.fillStyle = "#fff";
                ctx.font = "14px sans-serif";
                ctx.textAlign = "center";
                ctx.fillText(el.label, x, y + 5);
            }
        } else if (et.type === "route") {
            const pts = et.points;
            if (pts.length < 2) return;
            ctx.strokeStyle = color;
            ctx.lineWidth = 4;
            ctx.lineCap = "round";
            ctx.lineJoin = "round";
            ctx.beginPath();
            ctx.moveTo(pts[0].x, pts[0].y);
            for (let i = 1; i < pts.length; i++) ctx.lineTo(pts[i].x, pts[i].y);
            ctx.stroke();
            if (pts.length >= 2) {
                drawArrowhead(ctx, pts[pts.length - 2], pts[pts.length - 1], color);
            }
        } else if (et.type === "area") {
            const pts = et.points;
            if (pts.length < 3) return;
            ctx.fillStyle = colorToCssAlpha(el.color, fillOpacity);
            ctx.strokeStyle = color;
            ctx.lineWidth = 2;
            ctx.beginPath();
            ctx.moveTo(pts[0].x, pts[0].y);
            for (let i = 1; i < pts.length; i++) ctx.lineTo(pts[i].x, pts[i].y);
            ctx.closePath();
            ctx.fill();
            ctx.stroke();
        } else if (et.type === "arrow") {
            const end = et.end;
            ctx.strokeStyle = color;
            ctx.lineWidth = 4;
            ctx.lineCap = "round";
            ctx.beginPath();
            ctx.moveTo(pos.x, pos.y);
            ctx.lineTo(end.x, end.y);
            ctx.stroke();
            drawArrowhead(ctx, pos, end, color);
        } else if (et.type === "text") {
            ctx.fillStyle = color;
            ctx.font = `${et.font_size}px sans-serif`;
            ctx.fillText(et.content, pos.x, pos.y);
        } else if (et.type === "icon") {
            ctx.font = "24px sans-serif";
            ctx.fillText(ICON_EMOJI[et.icon_type] || "\u{1F4CD}", pos.x, pos.y);
        } else if (et.type === "drawing") {
            const pts = et.points;
            if (pts.length < 2) return;
            ctx.strokeStyle = color;
            ctx.lineWidth = et.stroke_width || 3;
            ctx.lineCap = "round";
            ctx.lineJoin = "round";
            ctx.beginPath();
            ctx.moveTo(pts[0].x, pts[0].y);
            for (let i = 1; i < pts.length; i++) ctx.lineTo(pts[i].x, pts[i].y);
            ctx.stroke();
        } else if (et.type === "ability") {
            ctx.strokeStyle = color;
            ctx.lineWidth = 2;
            ctx.beginPath();
            ctx.arc(pos.x, pos.y, 30, 0, Math.PI * 2);
            ctx.stroke();
            ctx.fillStyle = color;
            ctx.font = "12px sans-serif";
            ctx.textAlign = "center";
            ctx.fillText(et.ability_id, pos.x, pos.y + 4);
        }
    }

    function drawElementNumber(ctx, el, number) {
        const et = el.element_type;
        let bx = el.position.x - 10, by = el.position.y - 10;

        if (et.type === "player_marker") {
            bx = el.position.x + 18; by = el.position.y - 18;
        } else if (et.type === "route" || et.type === "area" || et.type === "drawing") {
            const pts = et.points;
            if (pts && pts.length > 0) { bx = pts[0].x - 10; by = pts[0].y - 10; }
        } else if (et.type === "text") {
            by = el.position.y - 20;
        }

        ctx.fillStyle = "rgba(0, 0, 0, 0.7)";
        ctx.beginPath();
        ctx.arc(bx, by, 9, 0, Math.PI * 2);
        ctx.fill();
        ctx.strokeStyle = "#ff6a00";
        ctx.lineWidth = 1.5;
        ctx.stroke();

        ctx.fillStyle = "#fff";
        ctx.font = "bold 10px sans-serif";
        ctx.textAlign = "center";
        ctx.textBaseline = "middle";
        ctx.fillText(String(number), bx, by);
        ctx.textBaseline = "alphabetic";
    }

    // =========================================================================
    // Canvas Renderer
    // =========================================================================

    const tileManager = new TileManager();
    const heroCache = new HeroImageCache();
    let lastRenderState = null;

    function getCtx(id) {
        const c = document.getElementById(id);
        return c ? c.getContext("2d") : null;
    }

    function renderBackground(state) {
        const ctx = getCtx("owc-bg");
        if (!ctx) return;
        const c = ctx.canvas;
        const { zoom, pan, mapId, metadata, selectedFloor, showHealthPacks } = state;

        ctx.imageSmoothingEnabled = false;
        ctx.fillStyle = "#1a1a2e";
        ctx.fillRect(0, 0, c.width, c.height);

        ctx.save();
        ctx.translate(pan.x, pan.y);
        ctx.scale(zoom, zoom);

        if (mapId) {
            const useTiles = metadata && metadata.tile_pyramid;

            if (useTiles) {
                renderBackgroundTiles(ctx, c, metadata, selectedFloor, zoom, pan, showHealthPacks);
            } else if (mapImage && mapImageId === mapId) {
                ctx.drawImage(mapImage, 0, 0);
                if (showHealthPacks && metadata) {
                    drawHealthPacks(ctx, metadata.health_packs || [], metadata);
                }
            } else {
                // Placeholder grid
                ctx.strokeStyle = "#333";
                ctx.lineWidth = 1;
                for (let x = 0; x < 5000; x += 100) {
                    ctx.beginPath(); ctx.moveTo(x, 0); ctx.lineTo(x, 5000); ctx.stroke();
                }
                for (let y = 0; y < 5000; y += 100) {
                    ctx.beginPath(); ctx.moveTo(0, y); ctx.lineTo(5000, y); ctx.stroke();
                }
                ctx.fillStyle = "#666";
                ctx.font = "24px sans-serif";
                ctx.fillText("Loading map...", 20, 40);
            }
        }

        ctx.restore();
    }

    function renderBackgroundTiles(ctx, canvas, metadata, floor, zoom, pan, showHP) {
        const pyramid = metadata.tile_pyramid;
        if (!pyramid) return;

        const floors = metadata.floors || [];
        const baseFloor = (floors.find(f => f.is_default) || floors.find(f => f.id === "ground") || floors[0]);
        const baseFloorId = baseFloor ? baseFloor.id : "ground";
        const selectedFloorId = floor || baseFloorId;
        const isOverlay = selectedFloorId !== baseFloorId;

        const vx = Math.max(0, -pan.x / zoom);
        const vy = Math.max(0, -pan.y / zoom);
        const vw = canvas.width / zoom;
        const vh = canvas.height / zoom;

        const visible = tileManager.visibleTiles(vx, vy, vw, vh, zoom, pyramid);

        // Render base floor (dimmed) if overlay
        if (isOverlay) {
            for (const [z, x, y] of visible) {
                const [dx, dy, dw, dh] = tileManager.tileRect(z, x, y, pyramid);
                const img = tileManager.getTile(baseFloorId, z, x, y);
                if (img) {
                    ctx.globalAlpha = 0.4;
                    ctx.drawImage(img, dx, dy, dw, dh);
                    ctx.globalAlpha = 1.0;
                } else {
                    ctx.fillStyle = "#2a2a3e";
                    ctx.fillRect(dx, dy, dw, dh);
                    if (!tileManager.isLoading(baseFloorId, z, x, y)) {
                        tileManager.loadTile(baseFloorId, z, x, y);
                    }
                }
            }
        }

        // Render selected floor
        for (const [z, x, y] of visible) {
            const [dx, dy, dw, dh] = tileManager.tileRect(z, x, y, pyramid);
            const img = tileManager.getTile(selectedFloorId, z, x, y);
            if (img) {
                ctx.drawImage(img, dx, dy, dw, dh);
            } else if (!isOverlay) {
                ctx.fillStyle = "#2a2a3e";
                ctx.fillRect(dx, dy, dw, dh);
            }
            if (!img && !tileManager.isLoading(selectedFloorId, z, x, y)) {
                tileManager.loadTile(selectedFloorId, z, x, y);
            }
        }

        if (showHP) {
            drawHealthPacks(ctx, metadata.health_packs || [], metadata);
        }
    }

    function renderElements(state) {
        const ctx = getCtx("owc-el");
        if (!ctx) return;
        const c = ctx.canvas;
        const { zoom, pan, elements, selectedPhase, fillOpacity } = state;

        const visible = elements.filter(
            e => !e.phase_id || e.phase_id === selectedPhase
        );

        ctx.clearRect(0, 0, c.width, c.height);
        ctx.save();
        ctx.translate(pan.x, pan.y);
        ctx.scale(zoom, zoom);

        const vx = -pan.x / zoom;
        const vy = -pan.y / zoom;
        const vw = c.width / zoom;
        const vh = c.height / zoom;

        for (let i = 0; i < visible.length; i++) {
            drawElement(ctx, visible[i], fillOpacity, heroCache);
            drawElementNumber(ctx, visible[i], i + 1);
        }

        ctx.restore();
    }

    function renderOverlay(state) {
        const ctx = getCtx("owc-ov");
        if (!ctx) return;
        const c = ctx.canvas;
        const { zoom, pan, drawColor, isDrawing, drawingPoints, arrowStart, arrowEnd,
                selectedElement, selectedPhase, elements } = state;

        ctx.clearRect(0, 0, c.width, c.height);
        ctx.save();
        ctx.translate(pan.x, pan.y);
        ctx.scale(zoom, zoom);

        // Drawing preview
        if (isDrawing && drawingPoints && drawingPoints.length > 0) {
            ctx.strokeStyle = colorToCss(drawColor);
            ctx.lineWidth = 3;
            ctx.lineCap = "round";
            ctx.lineJoin = "round";
            ctx.beginPath();
            ctx.moveTo(drawingPoints[0].x, drawingPoints[0].y);
            for (let i = 1; i < drawingPoints.length; i++) {
                ctx.lineTo(drawingPoints[i].x, drawingPoints[i].y);
            }
            ctx.stroke();
        }

        // Arrow preview
        if (arrowStart && arrowEnd) {
            ctx.strokeStyle = colorToCss(drawColor);
            ctx.lineWidth = 4;
            ctx.lineCap = "round";
            ctx.beginPath();
            ctx.moveTo(arrowStart.x, arrowStart.y);
            ctx.lineTo(arrowEnd.x, arrowEnd.y);
            ctx.stroke();
            drawArrowhead(ctx, arrowStart, arrowEnd, colorToCss(drawColor));
        }

        // Selection highlight
        if (selectedElement) {
            const vis = (elements || []).filter(
                e => !e.phase_id || e.phase_id === selectedPhase
            );
            const sel = vis.find(e => e.id === selectedElement);
            if (sel) {
                ctx.strokeStyle = "#00ff00";
                ctx.lineWidth = 2;
                ctx.setLineDash([4, 4]);
                ctx.beginPath();
                ctx.arc(sel.position.x, sel.position.y, 35, 0, Math.PI * 2);
                ctx.stroke();
                ctx.setLineDash([]);
            }
        }

        ctx.restore();
    }

    // =========================================================================
    // Public API (window.owCanvas)
    // =========================================================================

    window.owCanvas = {
        init(mapId) {
            tileManager.setMap(mapId || "");
            tileManager.onTileLoaded = () => {
                if (lastRenderState) renderBackground(lastRenderState);
            };
            if (mapId) {
                loadMapImage(mapId, () => {
                    if (lastRenderState) renderBackground(lastRenderState);
                });
            }
        },

        setMap(mapId) {
            tileManager.setMap(mapId || "");
            loadMapImage(mapId, () => {
                if (lastRenderState) renderBackground(lastRenderState);
            });
        },

        renderBackground(state) {
            lastRenderState = state;
            renderBackground(state);
        },

        renderElements(state) {
            renderElements(state);
        },

        renderOverlay(state) {
            renderOverlay(state);
        },

        renderAll(state) {
            lastRenderState = state;
            renderBackground(state);
            renderElements(state);
            renderOverlay(state);
        },

        getCanvasPos(clientX, clientY) {
            const c = document.getElementById("owc-ov");
            if (!c) return { x: 0, y: 0 };
            const rect = c.getBoundingClientRect();
            const scaleX = c.width / rect.width;
            const scaleY = c.height / rect.height;
            return { x: (clientX - rect.left) * scaleX, y: (clientY - rect.top) * scaleY };
        }
    };
})();
