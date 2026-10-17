# Extrude & Bevel, Inflate and Rotate

The other three 3D types of **Effect › 3D and Materials** next to [Revolve](revolve.md). Like Revolve each is a
live effect: the source path stays editable, the effect draws the shaded surface (with smooth shading, an image
shaded pixel by pixel and clipped to the surface's vector outline; otherwise shaded vector faces), and SVG/PDF
export, native saves and **Object › Expand Appearance** use the same evaluator.

Select a path or compound path, then choose **Effect › 3D and Materials › Extrude & Bevel…**, **Inflate…** or
**Rotate…**. The options dialog opens beside the artboard with tabs:

- **Object** (Extrude & Bevel and Inflate): the shape's own options, below.
- **Rotation**: a **Presets** menu, **Rotation X/Y/Z** (−180° to 180°) and **Perspective**. The rotation rings
  on the artboard turn the object about the centre of its bounds; Shift snaps to 15°.
- **Lighting**: **Shading**, the light's **Direction** and **Elevation** (or drag the light on the artboard),
  **Intensity** and **Ambient**, as for Revolve.

Preview changes are temporary until OK; Cancel restores the document; OK is one undo step. An object takes one
3D effect: edit its options in the Appearance panel, or remove it before applying another.

## Extrude & Bevel

The art is the front face; its outline is extruded backwards by **Depth** (0–2,000 pt), so seen straight on the
object is where the art was.

| Option | What it does |
|---|---|
| Depth | 0–2,000 pt. At 0 the object is flat (the art turned like Rotate). |
| Twist | 0° to 360°: the back face turned against the front, the walls following in slices. |
| Taper | 1–100 %: the back face's size against the front's. |
| Cap | **Solid** closes the front and back; **Hollow** leaves them open, showing the insides of the walls. |
| Bevel Shape | **None**, **Classic** (a straight chamfer), **Rounded** (convex quarter round), **Cove** (concave), **Step** (two steps) or **Ogee** (an S curve). Our own cross-sections. |
| Width, Height | How far the bevel reaches across the face and along the depth (0–1,000 pt each). The height is limited to the depth (half of it on each side with Bevel both sides). |
| Repeat, Space | The cross-section repeated 1–32 times across the width, with a share (0–90 %) of the width flat between repeats. |
| Bevel Inside | The bevel carved into the shape (the front face shrinks) instead of added around it (the walls stand outside the art). |
| Bevel both sides | The back is bevelled too. |

Holes stay holes, with walls of their own. Bevels move each corner along its bisector (mitres are limited to four
times the width), so a bevel wider than a narrow part of the shape folds over itself.

## Inflate

The shape puffed out like a cushion: its outline stays where the art is and the surface rises towards the
middle.

| Option | What it does |
|---|---|
| Depth | 0–2,000 pt of flat side between the front and the back. |
| Twist | 0° to 360°: the back turned against the front (its puff too), the sides following in slices. |
| Taper | 1–100 %: the back's size against the front's; a puffed back is lower in proportion. |
| Cap | **Solid** closes the back; **Hollow** leaves it off, so the inflated front is a shell open behind, its inside showing. The public documentation lists Cap for every 3D type without saying what it means for Inflate; this is our reading. |
| Volume | 0–100 %. The surface is a membrane under even pressure, held along the outline (and around holes): it rises vertically from the edge and rounds smoothly inside, without creases along the shape's middle. At 50 % a circle becomes a half sphere; 100 % puffs twice as high, 0 % is flat. |
| Both sides | The back puffs out too; otherwise it is flat. |

The surface is cut into a grid of 48 facets across the larger side (24 in a draft preview), halved or quartered where it rises steeply near the outline, each cut to the shape. A cell the outline leaves and comes back into is cut into separate parts, and neighbouring cells meet at exactly the same points along their shared sides, so the surface is watertight: no cracks or slivers along its rim. (Outlines whose subpaths cross each other are cut subpath by subpath and joined by the nonzero fill rule.) With smooth shading, each point's normal comes from the surface's height field itself (solved on a grid twice as fine as the facets, and near the outline from the exact distance to it), so mirror finishes reflect cleanly across the steep rim.

## Rotate

The flat art turned in 3D about the centre of its bounds. Every fill and stroke is turned with its own paint (a
stroke as its outline, so it foreshortens with the art) and shaded as one plane; a plane has two sides, so its
back shows when it is turned past 90°.

## Rotation presets

**Front**, **Back**, **Left**, **Right**, **Top** and **Bottom** face the art that way. **Off-Axis** tilts each 20°
down and 25° aside, so the top and the right show. **Isometric Left/Right/Top/Bottom** lay the art on that plane
of an isometric view (each visible axis foreshortened equally). These are our own values, from the geometry of
each view; the dialog shows a preset's name when the angles match it and **Custom** otherwise. The new effects
open at Off-Axis Front.

## Lighting and shadows

All four 3D types share the **Lighting** tab. The main light has a direction and elevation (also dragged on the
artboard), an intensity, a **Color** and a **Softness**: a soft light wraps further round the shape, so the side
turned away is not cut off sharply. **More Lights** adds up to three lights, each with its own direction,
elevation, intensity, colour and softness. Each face is shaded per colour channel: the ambient light plus every
light's share (`intensity × colour × (n·l + s) / (1 + s)`, where `s` is the softness), clamped to 0–1 and
multiplied into the object's colour. With smooth shading, gradients and patterns are multiplied by the shading
pixel by pixel (its highlight screened over them); without it they are darkened by each face's brightness only.

**Shadows** casts the solid's shadow: **Below Object** on a ground under it (tilted 20° towards the viewer, so it
shows in a front view), or **Behind Object** on a wall behind it, **Distance** further away. Its **Direction** is
**Straight** (new effects): the solid dropped straight onto the plane, whatever the lights, a soft footprint right
under it (or right behind it) as UI icon sets use; or **From Light**: cast along the main light, falling away
from it and longer as the light is lower (effects saved before the choice existed). Its **Opacity** and
**Softness** (a Gaussian blur, relative to the object's size) set how dark and how diffuse it is. Cast from the
light, there is no shadow when the light is below the ground or behind the wall. The shadow is drawn first, under
the faces, in the same group. The model and the defaults are our own.

**Contact Shadow** (0–100 %, 60 % for new effects) adds a dark, tight shadow where the object meets that plane:
the parts of the solid within 8 % of its size of the plane, dropped straight onto it, filled black at up to 85 %
of the setting with a small blur (3 % of the object's size), drawn over the cast shadow. It fades as **Distance**
lifts the object off the plane and is gone beyond that 8 %. It needs a shadow position: with **None** there is
no plane to meet.

**Presets** on the Lighting tab set the lights in one go: **Standard** (the defaults), **Studio** (a soft key
light, a cool fill and a back light), **Soft**, **Top Light**, **Dramatic** (a hard side light, a blue light
wrapping round the far edge and a faint cool fill, so the shadow side stays readable), **Warm & Cool** (a warm key light against cool ambient and fill) and **Flat**. **Ambient Color** tints the
ambient light. The tab shows a preset's name while every light matches it, **Custom** otherwise.

## Materials

The **Material** tab sets how the surface takes the light, for the clean 3D icons and components of UI work:

- **Highlight** adds a specular highlight per light (Blinn-Phong, `highlight × intensity × colour × (n·h)^k`),
  added on top of the paint, so a highlight can reach white. **Roughness** sets its spread: from a tight polished
  spot (exponent 256) to a broad sheen (exponent 2).
- **Metallic** turns the surface into metal: highlights take the paint's colour, the diffuse light fades, and it
  mirrors its surroundings in the paint's colour.
- **Reflections** picks those surroundings, our own procedural backdrops: **Studio** (for new effects: a light
  grey sweep with a darker floor, a large softbox above the key light, a tall strip on the right, a light
  ceiling and a soft fill behind the camera), **Outdoor** (a blue sky paling to the horizon over brown ground,
  the classic chrome look), **Sunset** (a warm horizon and a low sun under a violet sky), **Dark** (black, with
  thin bright strips either side and overhead) and **Gradient** (bright overhead to dark underfoot, what effects
  saved before this option reflect). The reflection is the view mirrored about the normal, blurred by the
  roughness. Glossy surfaces that aren't metal mirror the backdrop faintly too, most at grazing angles
  (Schlick's Fresnel term) and less the rougher they are; with **Gradient** they mirror nothing.
- **Rim Light** brightens the silhouette, where the surface turns away from the viewer (`(1 − n·v)³`), in its
  **Color**.
- **Cel Bands** (2–8) shades in that many flat bands with hard edges and a hard highlight, cel style; 0 is
  continuous. The light all the lights give together is banded, from the ambient light alone to full light, so
  more lights don't stack their bands' edges; a banded surface takes no glossy sheen.
- **Smooth shading** (on for new effects) shades the surface pixel by pixel: curved surfaces shade smoothly,
  flat faces take a gentle falloff, and highlights and cel band edges are as smooth as the pixels whatever the
  size of the faces. Each vertex has a normal: for Inflate the exact normal of its height field, elsewhere the
  area-weighted average of the faces around it, twice over, without crossing creases sharper than 50°. The faces
  are drawn into a depth buffer at twice the image's resolution each way, the normal and the position blended
  across each face (mean value coordinates, so a face never creases inside), every pixel shaded with the lights,
  which with the eye sit at six times the object's size. The image (up to 768 pixels along its longer side, at
  most 8 pixels a point) is clipped to the union of the faces, so the silhouette stays a crisp vector edge. With
  a paint that isn't a flat colour (a gradient, a pattern) the faces stay vector, each shaded with its mean.
  Effects saved before this option keep their flat vector faces until it is turned on.
- **Ambient Occlusion** (0–100 %, 60 % for new effects; needs smooth shading) darkens creases and hollows, where
  the surface hides part of its surroundings from itself: the cleft of a heart, the inside of a ring. From each
  pixel's normal and position, 32 samples spiral out to a quarter of the object's half size; a sample whose point
  rises above the pixel's tangent plane hides some of the sky, less the farther it is. The ambient light and
  reflections dim by what is hidden, the lights by half as much. It sees only the visible surface (screen-space
  ambient occlusion): what a nearer part hides behind it doesn't count.

While a value is being dragged in the dialog (a slider, a rotation ring, the light), the live preview shades a
draft: an image a third as fine each way (256 pixels along its longer side), Inflate on a grid half as fine and
Revolve in half the segments, so the preview follows the pointer (about 38 ms against 240 ms for a glossy
inflated heart on four cores). Letting go shades it in full again; OK never keeps the draft. The silhouette stays
the crisp vector outline throughout. The last few 3D arts are remembered by their object's content, so drawing
an object and finding its bounds shade it once.

**Presets**: **Matte** (the defaults), **Soft Touch**, **Plastic**, **Glossy**, **Metal**, **Chrome**, **Rim Glow**
and **Toon**, shown by name while the material matches. Each also sets **Reflections**: **Metal** reflects the
plain **Gradient**, which keeps it bright however it is turned, **Chrome** and the rest the **Studio**. **Toon** is
three bands with a small, hard highlight. All values are our own.

## Parameters (`effect.apply`, `effect.setParams`)

`threeD.extrude`: `depth`, `twist`, `taper` (%), `cap`, `bevel` (`none`, `classic`, `rounded`, `cove`, `step`,
`ogee`), `bevelWidth`, `bevelHeight`, `bevelRepeat`, `bevelSpace` (%), `bevelInside`, `bevelBothSides`.
`threeD.inflate`: `depth`, `volume` (%), `bothSides`. All three take the view and lighting parameters Revolve
uses: `rotationX`, `rotationY`, `rotationZ`, `perspective` (%), `shade`, `lightAzimuth`, `lightElevation`,
`lightIntensity` (%), `ambient` (%), and the lighting and shadow parameters every 3D effect takes: `lightColor`
(`#rrggbb`), `lightSoftness` (%), `lights` (up to three `{azimuth, elevation, intensity, color, softness}`),
`shadow` (`none`, `below`, `behind`), `shadowDirection` (`straight`, `light`; absent: `light`), `shadowDistance`,
`shadowOpacity` (%) and `shadowSoftness` (%), and the
material: `specular`, `roughness`, `metallic`, `rimLight` (%), `rimColor`, `shadingSteps` (0–8), `ambientColor`
and `smoothShading` (absent: off), `ambientOcclusion` (%, absent: 0), `environment` (`gradient`, `studio`,
`outdoor`, `sunset`, `dark`; absent: `gradient`) and `contactShadow` (%, absent: 0). New effects store each
option on (smooth shading, 60 % occlusion, a 60 % contact shadow, the studio and a straight shadow); an effect saved before an
option existed draws without it, as it did then. `draftPreview` (true) asks for the quick draft the dialog
shows while a value is dragged. Lengths are points and scale with the object (Scale Strokes &
Effects).
Commands validate the shape and the parameters before the document changes.

## What differs from the public documentation

Behaviour reference: the public [3D and Materials panel options](https://helpx.adobe.com/illustrator/desktop/special-effects-styles/create-3d-graphics/3d-materials-panel-options.html)
and [Create 3D objects](https://helpx.adobe.com/illustrator/desktop/special-effects-styles/create-3d-graphics/create-3d-objects.html)
pages, and the archived Extrude & Bevel option list (depth 0–2,000, cap on/off, bevel height, bevel extent in or
out, perspective). No Adobe software, assets, presets or bevel shapes were used.

- **Materials** are our own lighting model (highlight, roughness, metallic, rim light, cel bands, procedural
  reflection backdrops, screen-space ambient occlusion): there are no Adobe Substance materials, no textures,
  image-based lighting or bump maps, no graphics mapped onto surfaces and no **ray-traced rendering**, so a
  surface never reflects itself or other objects. Faces are painted with the object's top fill (else its top
  stroke) for Extrude & Bevel and Inflate; gradients and patterns are multiplied by the shading, not tinted by
  the lights' colours beyond that. The light gizmo moves the main light only.
- Smooth shading is an image: zoomed in far past its resolution, the shading (not the outline) softens, and
  exported SVG and PDF carry it as an embedded PNG. Turn **Smooth shading** off for all-vector faces.
- Shadows are a blurred projection of the solid on one plane; they don't fall on the object itself or on other
  objects.
- **Perspective** is a 0–100 % strength, as Revolve's, not a 0–160° lens angle.
- The bevel shapes and their names are our own; custom bevel paths are not supported.
- Visibility is a painter's algorithm: faces of a closed solid turned away are left out, the rest drawn back to
  front. Concave solids seen at steep angles can overlap wrongly; open (hollow) solids show both sides.
- Without smooth shading, shading is flat per facet: an inflated rim seen almost edge-on shows faint banding
  where the facets get finer.
- Paths and compound paths only: convert type, images and groups to outlines first. One 3D effect per object.
- Curves are followed within 1/2000 of the shape's size (at least 0.05 pt), so a small curve takes a few points
  and a large one up to 32. Extrude & Bevel and Inflate take up to 2,048 outline points, Rotate (which only turns
  the outline) 65,536.
- **3D (Classic)** (the track cube, surface types, blend steps, map art), Turntable and 3D export are separate,
  unbuilt items.
