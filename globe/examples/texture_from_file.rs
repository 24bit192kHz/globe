use globe::{CameraConfig, Canvas, GlobeConfig};

fn main() {
    // use config builder to create a new globe struct
    let mut globe = GlobeConfig::new()
        // specify path to the texture file
        .with_texture_at("textures/earth.txt", None)
        // for built-in textures, add GlobeTemplate to the imports above and
        // use it here instead
        //.use_template(GlobeTemplate::Earth)
        .with_camera(CameraConfig::default())
        .build();

    // create a new canvas, sized in character cells
    let mut canvas = Canvas::new(120, 60, None);

    // render the globe onto the canvas
    globe.render_on(&mut canvas);

    // print out the canvas
    for y in 0..canvas.get_size().1 {
        let row: String = canvas.row(y).iter().collect();
        println!("{}", row);
    }
}
