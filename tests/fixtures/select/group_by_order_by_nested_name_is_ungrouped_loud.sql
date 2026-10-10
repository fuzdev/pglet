-- The nested-name rule reaching an *aggregated* query, where binding the key to the
-- output instead of the base column does not merely reorder rows — it decides whether
-- the statement is legal at all. `q` is the alias of `count(*)` and also a base column
-- of `ongb`; the nested spelling binds to the base column, which is not grouped, so
-- 18.6 is `42803`. pglet bound it to the aggregate's output and answered rows.
--
-- Its accept polarity is the whole-key `ORDER BY q`, which both engines answer by
-- sorting on the count.
-- @setup
CREATE TABLE ongb (q INT, a INT);
INSERT INTO ongb VALUES (3, 100), (1, 200), (2, 300);

-- @expect-error: column q must appear in the GROUP BY clause (42803)
-- @test
SELECT count(*) AS q FROM ongb GROUP BY a ORDER BY q+0;
