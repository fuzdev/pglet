-- `CopyGetAttnums` interleaves its two verdicts one list item at a time, as
-- `checkInsertTargets` does: a repeat at item 2 outranks an unknown name at item 3, so
-- this is `42701`. pglet resolved the whole list first and answered `42703`.
--
-- Its inverse is `unknown_column_beats_a_later_duplicate_to_loud.sql`. The pair is only
-- an assertion because this suite compares the exact SQLSTATE — the two answers share the
-- class `42`, so a class-level comparison scores both files green either way.
-- @setup
CREATE TABLE cdti (a int, b int);
INSERT INTO cdti VALUES (1, 2);
-- @copy-to
COPY cdti (a, a, zz) TO STDOUT;
