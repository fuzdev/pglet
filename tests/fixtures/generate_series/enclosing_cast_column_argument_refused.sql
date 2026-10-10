-- A column under a cast types the argument as written, so it is refused as a column
-- reference rather than read: read once against the enclosing row, the counts below
-- would be `2`, `4` where 18.6 reads the sibling's `n` (pglet-stricter).
-- @setup
CREATE TABLE gs_oc (id INT, n INT);
CREATE TABLE gs_ic (n INT);
INSERT INTO gs_oc VALUES (1, 1), (2, 2);
INSERT INTO gs_ic VALUES (1), (2);
-- @expect-error: generate_series with a column reference
-- @test
SELECT o.id, (SELECT count(*) FROM gs_ic, generate_series(1, n::int) g) FROM gs_oc o ORDER BY 1;
