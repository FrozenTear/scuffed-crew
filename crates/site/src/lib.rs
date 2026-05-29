// Components/pages built ahead of the routes that consume them; preserve the
// in-progress work rather than deleting it to satisfy the dead_code lint.
#![allow(dead_code)]

pub mod app;
pub mod components;
pub mod pages;
pub mod sections;

pub use app::App;

use wasm_bindgen::prelude::*;

#[wasm_bindgen(start)]
pub fn main() {
    console_error_panic_hook::set_once();
    tracing_wasm::set_as_global_default();
    tracing::info!("Scuffed Crew site initialized");
    leptos::mount::mount_to_body(App);
    setup_effects();
}

/// Sets up interactive effects that survive client-side route changes.
/// Uses MutationObserver to auto-observe new [data-reveal] elements and
/// re-attach card glow / hero spotlight on DOM changes.
fn setup_effects() {
    let _ = js_sys::eval(
        r###"requestAnimationFrame(function() {
            // ── Scroll Reveal (auto-observes new elements via MutationObserver) ──
            var revealObs = new IntersectionObserver(function(entries) {
                entries.forEach(function(entry) {
                    if (entry.isIntersecting) {
                        entry.target.classList.add('revealed');
                        revealObs.unobserve(entry.target);
                    }
                });
            }, { threshold: 0.15 });

            function observeReveals(root) {
                (root || document).querySelectorAll('[data-reveal]:not(.revealed)').forEach(function(el) {
                    revealObs.observe(el);
                });
            }
            observeReveals();

            // ── Active Nav ──
            var navObs = new IntersectionObserver(function(entries) {
                entries.forEach(function(entry) {
                    if (entry.isIntersecting) {
                        var id = entry.target.id;
                        document.querySelectorAll('.nav-links a').forEach(function(a) {
                            a.classList.remove('nav-active');
                        });
                        var link = document.querySelector('.nav-links a[href="/#' + id + '"]');
                        if (link) link.classList.add('nav-active');
                    }
                });
            }, { threshold: 0.3, rootMargin: '-60px 0px 0px 0px' });

            function observeSections() {
                document.querySelectorAll('section[id]').forEach(function(el) {
                    navObs.observe(el);
                });
            }
            observeSections();

            // ── Card Glow ──
            var glowAttached = new WeakSet();
            function attachCardGlow() {
                document.querySelectorAll('.pillar, .team-card, .comm-card').forEach(function(el) {
                    if (glowAttached.has(el)) return;
                    glowAttached.add(el);
                    el.addEventListener('mousemove', function(ev) {
                        var rect = el.getBoundingClientRect();
                        el.style.setProperty('--card-glow-x', (ev.clientX - rect.left) + 'px');
                        el.style.setProperty('--card-glow-y', (ev.clientY - rect.top) + 'px');
                    });
                });
            }
            attachCardGlow();

            // ── Hero Spotlight ──
            var spotAttached = false;
            function attachHeroSpotlight() {
                if (spotAttached) return;
                var hero = document.querySelector('.hero');
                var spot = document.querySelector('.hero-spotlight');
                if (hero && spot) {
                    spotAttached = true;
                    hero.addEventListener('mousemove', function(ev) {
                        var rect = hero.getBoundingClientRect();
                        spot.style.left = (ev.clientX - rect.left) + 'px';
                        spot.style.top = (ev.clientY - rect.top) + 'px';
                    });
                }
            }
            attachHeroSpotlight();

            // ── MutationObserver: re-run on DOM changes (route changes) ──
            new MutationObserver(function() {
                observeReveals();
                observeSections();
                attachCardGlow();
                attachHeroSpotlight();
            }).observe(document.body, { childList: true, subtree: true });
        })"###,
    );
}
