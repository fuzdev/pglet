-- `JOIN/ON` is the one clause-root gate that outranks the target list, so this is
-- also where that position is pinned.
-- @setup
CREATE TABLE bcjn (a INT);
INSERT INTO bcjn VALUES (9);
-- @expect-error: argument of JOIN/ON must be type boolean
-- @test
SELECT count(*) FROM bcjn x JOIN bcjn y ON x.a;
