//! # hyper-render
//!
//! A Chromium-free HTML rendering engine for generating PNG and PDF outputs.
//!
//! This library provides a simple, high-level API for rendering HTML content to
//! images (PNG) or documents (PDF) without requiring a browser or Chromium dependency.
//! It leverages the [Blitz](https://github.com/DioxusLabs/blitz) rendering engine
//! for HTML/CSS parsing and layout.
//!
//! ## Features
//!
//! - **PNG output**: Render HTML to PNG images using CPU-based rendering
//! - **PDF output**: Render HTML to PDF documents with vector graphics
//! - **No browser required**: Pure Rust implementation, no Chromium/WebKit
//! - **CSS support**: Flexbox, Grid, and common CSS properties via Stylo
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use hyper_render::{render, Config, OutputFormat};
//!
//! let html = r#"
//!     <html>
//!         <body style="font-family: sans-serif; padding: 20px;">
//!             <h1 style="color: navy;">Hello, World!</h1>
//!             <p>Rendered without Chromium.</p>
//!         </body>
//!     </html>
//! "#;
//!
//! // Render to PNG
//! let png_bytes = render(html, Config::default())?;
//! std::fs::write("output.png", png_bytes)?;
//!
//! // Render to PDF
//! let pdf_bytes = render(html, Config::default().format(OutputFormat::Pdf))?;
//! std::fs::write("output.pdf", pdf_bytes)?;
//! # Ok::<(), hyper_render::Error>(())
//! ```
//!
//! ## Configuration
//!
//! Use [`Config`] to customize the rendering:
//!
//! ```rust,no_run
//! use hyper_render::{Config, OutputFormat};
//!
//! let config = Config::new()
//!     .width(1200)
//!     .height(800)
//!     .scale(2.0)  // 2x resolution for retina displays
//!     .format(OutputFormat::Png);
//! ```

mod config;
mod error;
mod render;

pub use config::{ColorScheme, Config, OutputFormat};
pub use error::{Error, Result};

use blitz_dom::DocumentConfig;
use blitz_html::HtmlDocument;
use blitz_traits::shell::Viewport;

/// Render HTML content to the specified output format.
///
/// This is the main entry point for rendering HTML. It parses the HTML,
/// computes styles and layout, and renders to the format specified in the config.
///
/// # Arguments
///
/// * `html` - The HTML content to render
/// * `config` - Rendering configuration (dimensions, format, scale, etc.)
///
/// # Returns
///
/// Returns the rendered output as bytes (PNG image data or PDF document).
///
/// # Errors
///
/// Returns an error if:
/// - Configuration is invalid (zero dimensions, non-positive scale)
/// - HTML parsing fails
/// - Layout computation fails
/// - Rendering fails
/// - The requested output format feature is not enabled
///
/// # Example
///
/// ```rust,no_run
/// use hyper_render::{render, Config, OutputFormat};
///
/// let html = "<h1>Hello</h1>";
///
/// // PNG output (default)
/// let png = render(html, Config::default())?;
///
/// // PDF output
/// let pdf = render(html, Config::default().format(OutputFormat::Pdf))?;
/// # Ok::<(), hyper_render::Error>(())
/// ```
pub fn render(html: &str, config: Config) -> Result<Vec<u8>> {
    // Validate configuration
    config.validate()?;

    // Parse HTML and create document
    let (mut document, rx) = create_document(html, &config)?;

    // Process all resources (like data:image URIs) that were fetched during parsing
    while let Ok(resource) = rx.try_recv() {
        document.load_resource(resource);
    }

    // Resolve styles and compute layout
    document.resolve(0.0);

    // Render to the specified format
    match config.format {
        OutputFormat::Png => render::png::render_to_png(&document, &config),
        OutputFormat::Pdf => render::pdf::render_to_pdf(&document, &config),
    }
}

/// Render HTML content to PNG format.
///
/// Convenience function that renders directly to PNG without needing to specify
/// the format in the config.
///
/// # Example
///
/// ```rust,no_run
/// use hyper_render::{render_to_png, Config};
///
/// let png_bytes = render_to_png("<h1>Hello</h1>", Config::default())?;
/// std::fs::write("output.png", png_bytes)?;
/// # Ok::<(), hyper_render::Error>(())
/// ```
#[cfg(feature = "png")]
pub fn render_to_png(html: &str, config: Config) -> Result<Vec<u8>> {
    render(html, config.format(OutputFormat::Png))
}

/// Render HTML content to PDF format.
///
/// Convenience function that renders directly to PDF without needing to specify
/// the format in the config.
///
/// # Example
///
/// ```rust,no_run
/// use hyper_render::{render_to_pdf, Config};
///
/// let pdf_bytes = render_to_pdf("<h1>Hello</h1>", Config::default())?;
/// std::fs::write("output.pdf", pdf_bytes)?;
/// # Ok::<(), hyper_render::Error>(())
/// ```
#[cfg(feature = "pdf")]
pub fn render_to_pdf(html: &str, config: Config) -> Result<Vec<u8>> {
    render(html, config.format(OutputFormat::Pdf))
}

use blitz_dom::net::Resource;
use blitz_traits::net::{NetProvider, Request, BoxedHandler, Bytes};
use std::sync::mpsc::{channel, Sender, Receiver};
use std::sync::Arc;

struct DataUrlNetProvider {
    tx: Sender<Resource>,
}

impl NetProvider<Resource> for DataUrlNetProvider {
    fn fetch(&self, doc_id: usize, request: Request, handler: BoxedHandler<Resource>) {
        println!("FETCH CALLED: {}", request.url);
        if request.url.scheme() == "data" {
            let path = request.url.path();
            if let Some(comma_idx) = path.find(',') {
                let metadata = &path[..comma_idx];
                let data_str = &path[comma_idx + 1..];
                if metadata.contains(";base64") {
                    use base64::{Engine as _, engine::general_purpose};
                    match general_purpose::STANDARD.decode(data_str) {
                        Ok(decoded) => {
                            println!("SUCCESSFULLY DECODED BASE64 (len: {})", decoded.len());
                            let tx = self.tx.clone();
                            let callback = Arc::new(move |_doc_id: usize, result: std::result::Result<Resource, Option<String>>| {
                                println!("CALLBACK CALLED WITH RESULT: {}", result.is_ok());
                                if let Ok(res) = result {
                                    let _ = tx.send(res);
                                }
                            });
                            handler.bytes(doc_id, Bytes::from(decoded), callback);
                        }
                        Err(e) => {
                            eprintln!("Failed to decode base64 data URI: {}", e);
                            let tx = self.tx.clone();
                            let callback = Arc::new(move |_doc_id: usize, result: std::result::Result<Resource, Option<String>>| {
                                if let Ok(res) = result {
                                    let _ = tx.send(res);
                                }
                            });
                            // Provide an empty byte array to prevent hanging
                            handler.bytes(doc_id, Bytes::from(vec![]), callback);
                        }
                    }
                }
            }
        } else {
            let tx = self.tx.clone();
            let callback = Arc::new(move |_doc_id: usize, result: std::result::Result<Resource, Option<String>>| {
                if let Ok(res) = result {
                    let _ = tx.send(res);
                }
            });
            handler.bytes(doc_id, Bytes::from(vec![]), callback);
        }
    }
}

/// Create and configure a Blitz document from HTML.
fn create_document(html: &str, config: &Config) -> Result<(HtmlDocument, Receiver<Resource>)> {
    let viewport = Viewport::new(
        config.width,
        config.height,
        config.scale,
        config.color_scheme.into(),
    );

    let (tx, rx) = channel();
    let provider = Arc::new(DataUrlNetProvider { tx });

    let mut doc_config = DocumentConfig {
        viewport: Some(viewport),
        net_provider: Some(provider),
        ..Default::default()
    };
    
    if !config.fonts.is_empty() {
        let mut font_ctx = parley::FontContext::default();
        for font_data in &config.fonts {
            font_ctx.collection.register_fonts(font_data.clone().into(), None);
        }
        doc_config.font_ctx = Some(font_ctx);
    }

    Ok((HtmlDocument::from_html(html, doc_config), rx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_builder() {
        let config = Config::new()
            .width(1920)
            .height(1080)
            .scale(2.0)
            .format(OutputFormat::Png);

        assert_eq!(config.width, 1920);
        assert_eq!(config.height, 1080);
        assert_eq!(config.scale, 2.0);
    }

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.width, 800);
        assert_eq!(config.height, 600);
        assert_eq!(config.scale, 1.0);
    }

    #[test]
    fn test_config_validation_zero_width() {
        let config = Config::new().width(0);
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_validation_zero_height() {
        let config = Config::new().height(0);
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_validation_zero_scale() {
        let config = Config::new().scale(0.0);
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_validation_negative_scale() {
        let config = Config::new().scale(-1.0);
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_validation_valid() {
        let config = Config::new()
            .width(Config::MIN_DIMENSION)
            .height(Config::MIN_DIMENSION)
            .scale(0.1);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_config_fonts() {
        let config = Config::new().font(vec![0, 1, 2, 3]);
        assert_eq!(config.fonts.len(), 1);
        assert_eq!(config.fonts[0], vec![0, 1, 2, 3]);
    }

    #[test]
    fn test_font_injection_render() {
        let html = r#"<p style="font-family: CustomFont">Test</p>"#;
        let config = Config::default().font(vec![0, 1, 2, 3]).width(100).height(100);
        let result = render(html, config);
        result.unwrap();
    }

    #[test]
    fn test_data_uri_net_provider() {
        let html = r#"<img src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=" />"#;
        let result = render(html, Config::default().width(100).height(100));
        result.unwrap();
    }
}
