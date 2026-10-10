-- A window query's projected column is described by the column its **qualifier** names.
-- Both FROM relations carry a `k`, `wqn`'s an `int4` and `wqt`'s a `text`; `b.k` is
-- `wqt`'s. The planner's window arm looked the column up by bare name and took the first
-- `k` in the input — `wqn`'s — so `Describe` declared `int4` over `text` bytes while the
-- simple-query leg, which resolves the qualifier, declared `text`. The declared-type
-- comparison on the extended leg is what sees it; the rows agree either way.
--
-- `a.k` is the polarity pin: it is the first `k`, so a bare-name lookup got it right.
-- @setup
CREATE TABLE wqn (k INT, id INT);
CREATE TABLE wqt (k TEXT, id INT);
INSERT INTO wqn VALUES (1, 1);
INSERT INTO wqt VALUES ('x', 1), ('y', 2);

-- @test
SELECT a.k AS ak, b.k, row_number() OVER (ORDER BY b.id) AS rn FROM wqn a, wqt b ORDER BY b.id;
