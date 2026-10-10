-- The `ADD COLUMN` route, and the one kind of gate the other two routes cannot show:
-- a **backfill** violation. 18.6 evaluates the added column's default for each
-- existing row and checks the inline constraint against it, so this is `23514` there
-- — and now here, where the render probe used to answer `0A000` from inside the cook,
-- three steps earlier than the backfill it has to lose to.
--
-- The CHECK is unrenderable *and* violated by the backfilled value, which is what
-- makes the two gates co-fire on one statement; drop the `DEFAULT` and 18.6 fills
-- NULL, the predicate goes NULL, and nothing is violated.
-- @expect-error: the backfilled row violating the CHECK, not the interval literal (23514)
-- @setup
CREATE TABLE iv_ac_bf (n INT);
INSERT INTO iv_ac_bf VALUES (1);
-- @test
ALTER TABLE iv_ac_bf ADD COLUMN t TIMESTAMPTZ DEFAULT '2000-01-01' CHECK (t > now() - interval '1 day');
