use super::*;

const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAYAAAD0In+KAAAAEUlEQVR4nGP8z8Dwn4GBgQEADQUCAOAHawIAAAAASUVORK5CYII=";

#[test]
fn embedded_png_becomes_bitmap_filled_shape_and_preserves_vectors() {
    let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="50">
       <rect id="vector" width="5" height="5" fill="red"/>
       <image id="raster" x="10" y="20" width="20" height="10" preserveAspectRatio="none" href="data:image/png;base64,{PNG}"/>
    </svg>"#);
    let drawing = import_with_images(svg.as_bytes()).expect("valid embedded image SVG");
    assert!(drawing.report.skipped.is_empty(), "{:?}", drawing.report.skipped);
    assert_eq!(drawing.bitmaps.len(), 1);
    assert_eq!((drawing.bitmaps[0].bitmap.width, drawing.bitmaps[0].bitmap.height), (2, 1));
    assert_eq!(drawing.elements().count(), 2);
    let raster = drawing.elements().last().expect("image element");
    assert_eq!(raster.name, "raster");
    let ElementKind::Shape(shape) = &raster.kind else { panic!("image is not an editable shape"); };
    assert!(matches!(shape.fill.as_ref(), Some(Paint::Bitmap { repeat: false, .. })));
    assert_eq!(raster.transform.apply(Vec2::new(0.0, 0.0)), Vec2::new(10.0, 20.0));
    assert_eq!(raster.transform.apply(Vec2::new(2.0, 1.0)), Vec2::new(30.0, 30.0));
}

#[test]
fn original_svg_import_does_not_change_without_opt_in() {
    let svg = format!(r#"<svg><image width="2" height="1" href="data:image/png;base64,{PNG}"/><rect width="5" height="6"/></svg>"#);
    let imported = import(svg.as_bytes()).expect("valid SVG");
    assert!(imported.bitmaps.is_empty());
    assert!(imported.report.skipped.contains_key("images"));
    assert_eq!(imported.elements().count(), 1);
}

#[test]
fn embedded_image_referenced_by_use_is_imported() {
    let svg = format!(r##"<svg xmlns:xlink="http://www.w3.org/1999/xlink"><defs>
      <image id="img" width="2" height="1" xlink:href="data:image/png;base64,{PNG}"/>
      </defs><use href="#img" x="4" y="7"/></svg>"##);
    let imported = import_with_images(svg.as_bytes()).expect("valid referenced image");
    assert_eq!(imported.bitmaps.len(), 1);
    assert_eq!(imported.elements().count(), 1);
    let elem = imported.elements().next().expect("image");
    assert_eq!(elem.transform.apply(Vec2::ZERO), Vec2::new(4.0, 7.0));
}
