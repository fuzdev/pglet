-- The `UPDATE` spelling of the refusal pinned on the `DELETE` side in
-- `delete/correlated_outer_ref_quoted_dot_column_loud.sql` — and a **write** where 18.6
-- refuses, so both rows must survive.
--
-- The two statements differ only in the identity their scope carries.
-- `execute_delete` passes the target's name; `collect_updated_simple` used to pass the
-- statement's *alias*, so an **unaliased** UPDATE handed `None` — and a `None` there does
-- not mean "unnamed", it means "a join, addressed by synthesized `relation.column`
-- strings", which is precisely the addressing `u1`'s quoted `"a.b"` column is
-- byte-identical to.
--
-- **This fixture must stay unaliased.** The aliased spelling of the identical statement
-- (`UPDATE u1 x SET id = 77 WHERE …`) is `42P01` on both engines and was so before the
-- fix — it is the discriminating control, and writing this fixture with an alias would
-- pin nothing. The three fixtures the dotted-tier gate arrived with are a `DELETE` and
-- two `SELECT`s, none of which reaches this call site at all.
--
-- **And `u1` must have no bare `b`**, which is what this fixture's refusal turns on: it
-- pins the *dotted tier*, so with a bare `b` beside the quoted column the reference used
-- to escape past that tier into a qualifier-blind bare match and write the row anyway.
-- Both are refused now — a qualifier no frame owns leaves the outer chain — and the
-- escape route has its own guards
-- (`delete/correlated_outer_ref_bogus_qualifier_loud.sql`, whose `zzz` needs no quoted
-- column at all, and the row-survival tests in
-- `crates/pglet/tests/scope_relation_identity.rs`).
-- `update/unaliased_bogus_qualifier_loud.sql` is the same missing identity reached
-- through the *other* resolver.
--
-- **The `eref` rule the aliased control above rests on is now enforced in both
-- spellings**, and the correlated one used to be a write: `UPDATE hw x SET id = 99 WHERE
-- EXISTS (SELECT 1 FROM u2b WHERE u2b.yv = hw.v)` was `UPDATE 1`, writing `99` over the
-- `hit` row, where 18.6 is `42P01 "hw"` with a HINT naming the alias `x`. It is `42P01`
-- here too, pinned by `update/correlated_outer_ref_alias_hides_the_table_name.sql`; the
-- `SimpleColumns` spelling (`UPDATE hw x SET id = 98 WHERE hw.id = 1`) is `42P01` on both
-- and `WHERE x.id = 2` answers on both. The aliased spelling cited two paragraphs up
-- remains a control for *this* statement, whose `a` matches nothing either way.
--
-- Controls measured against 18.6, `42P01` on both engines: `u2.yv = zzz.nosuch`, and
-- `u2.yv = zzz.b` — the latter ruling out the bare fallback, since `u1` carries no `b`.
-- @setup
CREATE TABLE u1 (id INT, "a.b" TEXT);
CREATE TABLE u2 (yv TEXT);
INSERT INTO u1 VALUES (1, 'hit'), (2, 'miss');
INSERT INTO u2 VALUES ('hit');

-- @expect-error: missing FROM-clause entry for table "a" (42P01)
-- @test
UPDATE u1 SET id = 99 WHERE EXISTS (SELECT 1 FROM u2 WHERE u2.yv = a.b);
