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

/// Defers all interactive effects to the next animation frame so the DOM is ready.
fn setup_effects() {
    let _ = js_sys::eval(
        r###"requestAnimationFrame(function() {
            // ── Scroll Reveal ──
            var revealObs = new IntersectionObserver(function(entries, observer) {
                entries.forEach(function(entry) {
                    if (entry.isIntersecting) {
                        entry.target.classList.add('revealed');
                        observer.unobserve(entry.target);
                    }
                });
            }, { threshold: 0.15 });
            document.querySelectorAll('[data-reveal]').forEach(function(el) {
                revealObs.observe(el);
            });

            // ── Active Nav ──
            var navObs = new IntersectionObserver(function(entries) {
                entries.forEach(function(entry) {
                    if (entry.isIntersecting) {
                        var id = entry.target.id;
                        document.querySelectorAll('.nav-links a').forEach(function(a) {
                            a.classList.remove('nav-active');
                        });
                        var link = document.querySelector('.nav-links a[href="#' + id + '"]');
                        if (link) link.classList.add('nav-active');
                    }
                });
            }, { threshold: 0.3, rootMargin: '-60px 0px 0px 0px' });
            document.querySelectorAll('section[id]').forEach(function(el) {
                navObs.observe(el);
            });

            // ── Card Glow ──
            document.querySelectorAll('.pillar, .team-card, .comm-card').forEach(function(el) {
                el.addEventListener('mousemove', function(ev) {
                    var rect = el.getBoundingClientRect();
                    el.style.setProperty('--card-glow-x', (ev.clientX - rect.left) + 'px');
                    el.style.setProperty('--card-glow-y', (ev.clientY - rect.top) + 'px');
                });
            });

            // ── Hero Spotlight ──
            var hero = document.querySelector('.hero');
            var spot = document.querySelector('.hero-spotlight');
            if (hero && spot) {
                hero.addEventListener('mousemove', function(ev) {
                    var rect = hero.getBoundingClientRect();
                    spot.style.left = (ev.clientX - rect.left) + 'px';
                    spot.style.top = (ev.clientY - rect.top) + 'px';
                });
            }
        })"###,
    );
}
