use super::escape;

pub(super) struct PinPreview {
    pub id: String,
    pub number: i64,
    pub card: String,
}

pub(super) fn index(account: i64, pins: &[PinPreview]) -> String {
    let mut items = String::new();
    for pin in pins {
        items.push_str(&format!(
            "<div class=\"tray__item tray__item--pin\" id=\"pin_{}\" data-navigable-list-target=\"item\">{}<form action=\"/{account}/cards/{}/pin\" method=\"post\"><input type=\"hidden\" name=\"_method\" value=\"delete\"><button class=\"tray__remove-pin-btn btn btn--circle borderless\" type=\"submit\" aria-label=\"Unpin this card\"><span class=\"icon icon--pinned\" aria-hidden=\"true\"></span><span class=\"for-screen-reader\">Unpin this card</span></button></form></div>",
            escape(&pin.id), pin.card, pin.number
        ));
    }
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Pinned | Rizzy</title><style>{}</style></head><body><header class=\"header\"><a href=\"/{account}/\">Back to Home</a><h1>Pinned</h1></header><main class=\"pins-list panel panel--wide flex center borderless unpad flex-column gap-half margin-block-start\"><turbo-frame id=\"pins\">{items}</turbo-frame></main></body></html>",
        include_str!("../static/pins.css")
    )
}
