//! Gesture ownership checks against the real native library.
#![cfg(not(miri))]

use libghostty_vt::{
    Error, Terminal,
    alloc::Allocator,
    selection::gesture::{
        AutoscrollTickEvent, Behavior, Behaviors, DeepPressEvent, DragEvent, Geometry, Gesture,
        PressEvent, ReleaseEvent,
    },
    terminal::{Point, PointCoordinate},
};

fn point(x: u16) -> Point {
    Point::Active(PointCoordinate { x, y: 0 })
}

fn geometry() -> Geometry {
    Geometry {
        columns: 8,
        cell_width: 10,
        padding_left: 0,
        screen_height: 100,
    }
}

fn rejects_other_terminal(gesture: &mut Gesture<'_>, terminal: &mut Terminal<'_, '_>) {
    assert!(matches!(gesture.anchor(terminal), Err(Error::InvalidValue)));
    assert!(matches!(
        gesture.click_count(terminal),
        Err(Error::InvalidValue)
    ));
    assert!(matches!(
        gesture.dragged(terminal),
        Err(Error::InvalidValue)
    ));
    assert!(matches!(
        gesture.autoscroll(terminal),
        Err(Error::InvalidValue)
    ));
    assert!(matches!(
        gesture.behavior(terminal),
        Err(Error::InvalidValue)
    ));
    assert!(matches!(gesture.reset(terminal), Err(Error::InvalidValue)));
    assert!(matches!(
        PressEvent::new()
            .unwrap()
            .apply(gesture, terminal, point(0)),
        Err(Error::InvalidValue)
    ));
    assert!(matches!(
        ReleaseEvent::new().unwrap().apply(gesture, terminal, None),
        Err(Error::InvalidValue)
    ));
    assert!(matches!(
        DragEvent::new()
            .unwrap()
            .apply(gesture, terminal, point(1), geometry()),
        Err(Error::InvalidValue)
    ));
    assert!(matches!(
        DeepPressEvent::new().unwrap().apply(gesture, terminal),
        Err(Error::InvalidValue)
    ));
    assert!(matches!(
        AutoscrollTickEvent::new().unwrap().apply(
            gesture,
            terminal,
            PointCoordinate { x: 1, y: 0 },
            geometry()
        ),
        Err(Error::InvalidValue)
    ));
}

#[test]
fn gestures_reject_other_terminals_without_losing_their_anchor() {
    let mut owner = Terminal::new(8, 2).unwrap();
    owner.vt_write(b"Alpha");
    let mut other = Terminal::new(8, 2).unwrap();
    other.vt_write(b"Bravo");
    let mut gesture = Gesture::new(&owner).unwrap();
    PressEvent::new()
        .unwrap()
        .apply(&mut gesture, &owner, point(0))
        .unwrap();

    rejects_other_terminal(&mut gesture, &mut other);
    // Binding follows the terminal, not the address of its Rust wrapper.
    std::mem::swap(&mut owner, &mut other);
    assert_eq!(
        gesture
            .anchor(&other)
            .unwrap()
            .unwrap()
            .cell()
            .unwrap()
            .codepoint()
            .unwrap(),
        u32::from('A')
    );
    assert_eq!(gesture.click_count(&other).unwrap(), 1);
    gesture.reset(&other).unwrap();
    assert!(gesture.anchor(&other).unwrap().is_none());
    assert!(matches!(gesture.anchor(&owner), Err(Error::InvalidValue)));
}

#[test]
fn gesture_can_outlive_its_terminal_but_cannot_use_a_replacement() {
    let allocator = Allocator::GLOBAL;
    let mut gesture = {
        let terminal = Terminal::new_with_alloc(&allocator, 8, 2).unwrap();
        let mut gesture = Gesture::new_with_alloc(&allocator, &terminal).unwrap();
        PressEvent::new()
            .unwrap()
            .apply(&mut gesture, &terminal, point(0))
            .unwrap();
        gesture
    };
    let replacement = Terminal::new(8, 2).unwrap();
    assert!(matches!(
        gesture.anchor(&replacement),
        Err(Error::InvalidValue)
    ));
    assert!(matches!(
        gesture.reset(&replacement),
        Err(Error::InvalidValue)
    ));
    // Drop must not dereference the dead terminal or its freed tracked pin.
    drop(gesture);
}

#[test]
fn events_resolve_current_cells_and_preserve_selection_behavior() {
    let mut terminal = Terminal::new(8, 2).unwrap();
    terminal.vt_write(b"Alpha");
    let mut gesture = Gesture::new(&terminal).unwrap();
    let mut press = PressEvent::new().unwrap();
    press
        .set_behaviors(&Behaviors::new().with_single_click_behavior(Behavior::Word))
        .unwrap();
    let selection = press
        .apply(&mut gesture, &terminal, point(2))
        .unwrap()
        .unwrap();
    assert_eq!(
        selection.start().cell().unwrap().codepoint().unwrap(),
        u32::from('A')
    );
    assert_eq!(
        selection.end().cell().unwrap().codepoint().unwrap(),
        u32::from('a')
    );
    terminal.set_selection(Some(&selection)).unwrap();
    ReleaseEvent::new()
        .unwrap()
        .apply(&mut gesture, &terminal, Some(point(2)))
        .unwrap();
    gesture.reset(&terminal).unwrap();
    terminal.vt_write(b"\x1bcBravo");
    let selection = press
        .apply(&mut gesture, &terminal, point(2))
        .unwrap()
        .unwrap();
    assert_eq!(
        selection.start().cell().unwrap().codepoint().unwrap(),
        u32::from('B')
    );
    assert_eq!(
        selection.end().cell().unwrap().codepoint().unwrap(),
        u32::from('o')
    );
    assert!(matches!(
        press.apply(&mut gesture, &terminal, point(8)),
        Err(Error::InvalidValue)
    ));
    assert_eq!(gesture.click_count(&terminal).unwrap(), 1);
    ReleaseEvent::new()
        .unwrap()
        .apply(&mut gesture, &terminal, None)
        .unwrap();
    gesture.reset(&terminal).unwrap();
}

#[test]
fn autoscroll_resolves_the_pointer_after_scrolling() {
    let mut terminal = Terminal::new(8, 2).unwrap();
    terminal.vt_write(b"AAA\r\nBBB\r\nCCC\r\nDDD");
    let mut gesture = Gesture::new(&terminal).unwrap();
    let top = PointCoordinate { x: 0, y: 0 };
    assert_eq!(
        terminal
            .grid_ref(Point::Viewport(top))
            .unwrap()
            .cell()
            .unwrap()
            .codepoint()
            .unwrap(),
        u32::from('C')
    );
    // An upward selection includes the anchor when the click is on its right
    // side (past the 60% cell threshold).
    PressEvent::new()
        .unwrap()
        .set_position(9.0, 50.0)
        .unwrap()
        .apply(&mut gesture, &terminal, Point::Viewport(top))
        .unwrap();
    let pointer = PointCoordinate { x: 2, y: 0 };
    DragEvent::new()
        .unwrap()
        .set_position(25.0, 0.0)
        .unwrap()
        .apply(
            &mut gesture,
            &terminal,
            Point::Viewport(pointer),
            geometry(),
        )
        .unwrap();
    let mut tick = AutoscrollTickEvent::new().unwrap();
    tick.set_position(25.0, 0.0).unwrap();
    let selection = tick
        .apply(&mut gesture, &mut terminal, pointer, geometry())
        .unwrap()
        .unwrap();
    assert_eq!(
        selection.start().cell().unwrap().codepoint().unwrap(),
        u32::from('C')
    );
    assert_eq!(
        selection.end().cell().unwrap().codepoint().unwrap(),
        u32::from('B')
    );
    assert_eq!(
        terminal
            .grid_ref(Point::Viewport(top))
            .unwrap()
            .cell()
            .unwrap()
            .codepoint()
            .unwrap(),
        u32::from('B')
    );
    gesture.reset(&terminal).unwrap();
}
