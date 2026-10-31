//! QR de pareamento: no terminal (cores Catppuccin Mocha) e em SVG para a interface.

use qrcode::render::svg;
use qrcode::render::unicode::Dense1x2;
use qrcode::QrCode;

const BASE: (u8, u8, u8) = (0x1e, 0x1e, 0x2e);
const TEXT: (u8, u8, u8) = (0xcd, 0xd6, 0xf4);

pub fn render(payload: &str) -> Option<String> {
  let code = QrCode::new(payload.as_bytes()).ok()?;
  // O Dense1x2 desenha os módulos claros como blocos: blocos em `text`
  // sobre fundo `base` dão o contraste certo para a câmera do celular.
  let art = code
    .render::<Dense1x2>()
    .dark_color(Dense1x2::Dark)
    .light_color(Dense1x2::Light)
    .quiet_zone(true)
    .build();
  let fg = format!("\x1b[38;2;{};{};{}m", TEXT.0, TEXT.1, TEXT.2);
  let bg = format!("\x1b[48;2;{};{};{}m", BASE.0, BASE.1, BASE.2);
  let reset = "\x1b[0m";
  Some(
    art
      .lines()
      .map(|line| format!("{fg}{bg}{line}{reset}"))
      .collect::<Vec<_>>()
      .join("\n"),
  )
}

/// SVG escuro sobre claro (o que a câmera lê melhor), para a tela de pareamento.
pub fn svg(payload: &str) -> Option<String> {
  let code = QrCode::new(payload.as_bytes()).ok()?;
  Some(
    code
      .render::<svg::Color<'_>>()
      .dark_color(svg::Color("#11111b"))
      .light_color(svg::Color("#ffffff"))
      .quiet_zone(true)
      .min_dimensions(256, 256)
      .build(),
  )
}
