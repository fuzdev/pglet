-- The join path resolved correctly only **by accident**: the join merge renames each
-- side's columns to `"table.name"`, which is exactly the spelling the old qualified
-- lookup demanded. A fix that stopped honoring that spelling would have broken the
-- one case that worked, so this is a positive lock on it — both sides' declared
-- types, including two same-named columns disambiguated only by their qualifier.
-- (Distinct `AS` labels are required, not cosmetic: the harness keys a row's values
-- by column name, so two columns both named `v` would collapse into one and the
-- value comparison would silently cover half of what it appears to.)
-- @setup
CREATE TABLE qjl (id INT PRIMARY KEY, v INT);
CREATE TABLE qjr (id INT PRIMARY KEY, v BOOL);
INSERT INTO qjl VALUES (1, 42);
INSERT INTO qjr VALUES (1, true);

-- @test
SELECT qjl.id, qjl.v AS lv, qjr.v AS rv FROM qjl JOIN qjr ON qjl.id = qjr.id;
