-- Same boundary on the other side: `transformOnConflictClause` runs after the row
-- source, so the column list's `42701` outranks the clause. pglet wrote `a = 7`.
-- @setup
CREATE TABLE dtco (a INT PRIMARY KEY, b INT);

-- @expect-error: column "a" specified more than once (42701)
-- @no-profile-lock: the refusal is raised while the column list is resolved, before the arbiter is inferred or any index selection runs
-- @test
INSERT INTO dtco (a, a) VALUES (6, 7) ON CONFLICT DO NOTHING;
