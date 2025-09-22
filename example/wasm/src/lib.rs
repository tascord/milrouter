use {
    dominator::{Dom, events, html},
    futures_signals::signal::{Mutable, SignalExt},
    milrouter::anyhow,
    wasm_bindgen::prelude::wasm_bindgen,
    wasm_bindgen_futures::spawn_local,
};

pub const LOADER_SVG: &str = r#"<svg  xmlns="http://www.w3.org/2000/svg"  width="24"  height="24"  viewBox="0 0 24 24"  fill="none"  stroke="currentColor"  stroke-width="2"  stroke-linecap="round"  stroke-linejoin="round"  class="icon icon-tabler icons-tabler-outline icon-tabler-loader-2"><path stroke="none" d="M0 0h24v24H0z" fill="none"/><path d="M12 3a9 9 0 1 0 9 9" /></svg>"#;

#[wasm_bindgen(start)]
fn main() { dominator::append_dom(&dominator::body(), page()); }

fn page() -> Dom {
    let resp = Mutable::<anyhow::Result<String>>::new(Ok(String::new()));
    let in_flight = Mutable::<bool>::default();

    html!("div", {
        .class(["w-screen", "h-screen", "flex", "flex-col", "items-center", "justify-center", "space-y-4"])
        .child(html!("pre", {
            .class(["p-8","bg-neutral-100","border","border-neutral-200","shadow-sm"])
            .text_signal(resp.signal_ref(|v| format!("{v:#?}")))
        }))
        .child(html!("button", {
            .class(["px-4","py-2","rounded-lg","shadow-sm","bg-blue-300","text-white","font-semibold","text-lg","disabled:bg-neutral-600","flex","space-x-2","items-center"])
            .prop_signal("disabled", in_flight.signal())
            .child_signal(in_flight.signal().map(|v| v.then_some(html!("svg", {
                .class(["mr-2","animate-spin"])
                .after_inserted(|dom| {
                    dom.set_outer_html(LOADER_SVG);
                })
            }))))
            .text("Submit")
             .event({
                let in_flight = in_flight.clone();
                let resp = resp.clone();
                move |_: events::Click| {
                    in_flight.set(true);
                    let in_flight = in_flight.clone();
                    let resp = resp.clone();
                    spawn_local(async move {
                     resp.set(milrouter::wasm::request(
                        server::EndpointTheTime,
                        () // No arg in Fn signature, so we use unit.
                    ).await);
                    in_flight.set(false);
                   });
                }
             })
        }))
    })
}
