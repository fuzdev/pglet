-- Where each clause-root gate sits is measured, not reasoned, and the placements
-- disagree with one another. `transformFromClause` runs before
-- `transformTargetList`, so `JOIN/ON`'s gate beats an unbindable projection item.
-- @setup
CREATE TABLE bcord1 (k INT);
CREATE TABLE bcord2 (k INT);
-- @expect-error: argument of JOIN/ON must be type boolean, not the target list's 42703
-- @test
SELECT nosuchcol FROM bcord1 a JOIN bcord2 c ON a.k;
