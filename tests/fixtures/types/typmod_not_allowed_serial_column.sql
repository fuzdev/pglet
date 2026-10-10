-- The `SERIAL` family is refused at its **rewritten** integer type: 18.6's grammar
-- has already turned `serial` into `integer` by the time parse analysis reads the
-- modifier, so the message names `integer`, not `serial`. Accepting it used to land a
-- bare nullable `integer` — no sequence, no `NOT NULL`, no `DEFAULT`.
-- @expect-error: serial(3) is refused, naming integer
-- @expect-message
-- @test
CREATE TABLE tmser (a serial(3));
