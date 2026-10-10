-- `UPDATE t SET … FROM s WHERE … RETURNING s.*` — the qualified wildcard names the
-- **source**, and answers the source's columns and the source's values.
--
-- The qualifier is the whole subject. This statement once reached the executor's
-- sole-`*` fast path, which returned the projection's input rows and columns *without
-- looking at the qualifier at all*, so it answered the **target**'s column names and the
-- target's values (measured: `id, tag` = `1, hit`) where 18.6 answers the source's
-- (`id, note` = `1, src`). No error, no NULLs, a plausible row: a client reading it by
-- name got a column that exists and a value from the wrong table.
--
-- The two relations differ in column *names* as well as values, so a build answering the
-- target's row fails on the shape rather than only on the data — and its control is
-- `from_returning_wildcard.sql`, the same spelling qualified by the *target*, which the
-- spine itself writes.
-- @setup
CREATE TABLE rsw_t (id INT, tag TEXT);
CREATE TABLE rsw_s (id INT, note TEXT);
INSERT INTO rsw_t VALUES (1, 'before');
INSERT INTO rsw_s VALUES (1, 'src');

-- @test
UPDATE rsw_t SET tag = 'hit' FROM rsw_s s WHERE s.id = rsw_t.id RETURNING s.*;
