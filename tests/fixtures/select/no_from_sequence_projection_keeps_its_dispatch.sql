-- The accept polarity of the **one exclusion** in the no-FROM tail routing. The executor
-- dispatches `SELECT nextval/currval/setval(…)` on `Plan::is_sequence_projection`, which
-- matches a bare `Plan::Values`, so wrapping such a projection in the `Sort`/`Limit` the
-- tail builds routes it to the read-only eval path instead — where a `SequenceFn` fails
-- loud `0A000` and this accepted statement becomes an error.
--
-- `plan_no_from_order_limit` therefore runs its gates and then declines to wrap. Nothing
-- loud becomes silent (`SELECT nextval(…) ORDER BY zzz` is still `42703` and `ORDER BY 5`
-- still `42P10`); what stays open is that the ordering and the bound are not applied —
-- `SELECT nextval(…) LIMIT 0` answers a row here against `(0 rows)` on 18.6, recorded in
-- `EDGES.md`. The `ORDER BY 1` spelling is used here because it
-- is the one whose *answer* both engines agree on.
-- @setup
CREATE SEQUENCE nfseq;
-- @test
SELECT nextval('nfseq') AS n ORDER BY 1;
