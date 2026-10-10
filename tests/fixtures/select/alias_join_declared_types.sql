-- The same defect over a **join of two aliases**, and the third discriminator: a
-- join has no single table to fall back to at all, so the walk's fallback key was
-- `None` here whether or not aliases were written. That means a fix keyed on "the
-- scan's real table name" leaves this red for a reason unrelated to the alias — and
-- an alias-map fix that resolves each qualifier to its base table fixes it while
-- still leaving the derived-table and CTE fixtures red (both measured). Only
-- resolving against the input relations covers all three.
--
-- Distinct `AS` labels are required, not cosmetic: the harness keys a row's values
-- by column name, so same-named columns would collapse into one (the same reason
-- `qualified_join_declared_types` gives).
--
-- `join_same_width` is the polarity pin.
-- @setup
CREATE TABLE mw (s SMALLINT, i INT, b BIGINT);
INSERT INTO mw VALUES (30000, 60000, 5000000000);

-- @test
SELECT m.s + n.b AS join_mixed,     -- int8 (was text)
       n.b + m.i AS join_wide_left, -- int8 (was text)
       m.i + m.i AS join_same_width -- int4 (was text) — polarity pin
FROM mw m JOIN mw n ON true;
