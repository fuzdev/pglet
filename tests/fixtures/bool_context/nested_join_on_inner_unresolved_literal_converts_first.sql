-- An inner `ON` holding an unresolved literal takes its conversion's `22P02` ahead of
-- the outer `ON`'s gate — so the inner verdict wins where *both* verdicts come from the
-- clause-root gate itself, and not only where the inner one comes from the resolve walk
-- beneath it.
-- @setup
CREATE TABLE bcnj7 (k INT, t TEXT);
CREATE TABLE bcnj8 (k INT, t TEXT);
CREATE TABLE bcnj9 (k INT, t TEXT);
-- @expect-error: invalid input syntax for type boolean: "abc", not the outer ON's 42804
-- @test
SELECT 1 FROM bcnj7 JOIN bcnj8 ON 'abc' JOIN bcnj9 ON bcnj8.k;
