/**
 * Kysely ORM tests for pglet compatibility.
 *
 * Self-contained: starts a pglet server of its own for the file.
 *
 * Tests basic CRUD operations using Kysely ORM against pglet.
 * Run: npm run test:kysely
 *
 * Mirrors the Drizzle test structure for consistent coverage.
 */

import { test } from "vitest";
import { assert_equal } from "../../lib/assert.ts";
import { Kysely, type Migration, Migrator, PostgresDialect, sql } from "kysely";
import pg from "pg";
import type { Database, NewUser, NewPost } from "./types.ts";
import { start_suite_server } from "../../lib/server.ts";

const { Pool } = pg;

// A server of this file's own, on a port the kernel picks, stopped when the file's tests end
const server = await start_suite_server();

const CONFIG = {
  host: server.host,
  port: server.port,
  user: "postgres",
  database: "postgres",
};

/**
 * Helper to create a fresh Kysely connection.
 * Data persists across connections (shared state).
 */
async function withDb<T>(
  fn: (db: Kysely<Database>) => Promise<T>,
): Promise<T> {
  const pool = new Pool(CONFIG);
  const db = new Kysely<Database>({
    dialect: new PostgresDialect({ pool }),
  });
  try {
    return await fn(db);
  } finally {
    await db.destroy();
  }
}

/**
 * Setup tables using raw SQL.
 * Drops tables first since data persists across connections.
 */
async function setupTables(db: Kysely<Database>): Promise<void> {
  // Drop existing tables first (data persists across connections)
  await sql`DROP TABLE IF EXISTS posts CASCADE`.execute(db);
  await sql`DROP TABLE IF EXISTS users CASCADE`.execute(db);
  await sql`
    CREATE TABLE users (
      id INT PRIMARY KEY,
      name TEXT NOT NULL,
      email TEXT NOT NULL,
      active BOOL DEFAULT true,
      created_at TIMESTAMP
    )
  `.execute(db);
  await sql`
    CREATE TABLE posts (
      id INT PRIMARY KEY,
      title TEXT NOT NULL,
      content TEXT,
      author_id INT NOT NULL,
      published BOOL DEFAULT false
    )
  `.execute(db);
}

// =============================================================================
// INSERT Tests
// =============================================================================

test("kysely - insert single row", async () => {
  await withDb(async (db) => {
    await setupTables(db);

    await db.insertInto("users").values({
      id: 1,
      name: "Alice",
      email: "alice@example.com",
    }).execute();

    const result = await db.selectFrom("users").selectAll().execute();
    assert_equal(result.length, 1);
    assert_equal(result[0].name, "Alice");
    assert_equal(result[0].email, "alice@example.com");
  });
});

test("kysely - insert multiple rows", async () => {
  await withDb(async (db) => {
    await setupTables(db);

    await db.insertInto("users").values([
      { id: 1, name: "Alice", email: "alice@example.com" },
      { id: 2, name: "Bob", email: "bob@example.com" },
      { id: 3, name: "Carol", email: "carol@example.com" },
    ]).execute();

    const result = await db.selectFrom("users").selectAll().execute();
    assert_equal(result.length, 3);
  });
});

// =============================================================================
// SELECT Tests
// =============================================================================

test("kysely - select all", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insertInto("users").values([
      { id: 1, name: "Alice", email: "alice@example.com", active: true },
      { id: 2, name: "Bob", email: "bob@example.com", active: false },
    ]).execute();

    const result = await db.selectFrom("users").selectAll().execute();
    assert_equal(result.length, 2);
  });
});

test("kysely - select with where eq", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insertInto("users").values([
      { id: 1, name: "Alice", email: "alice@example.com" },
      { id: 2, name: "Bob", email: "bob@example.com" },
    ]).execute();

    const result = await db.selectFrom("users").selectAll().where("id", "=", 1).execute();
    assert_equal(result.length, 1);
    assert_equal(result[0].name, "Alice");
  });
});

test("kysely - select with where gt", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insertInto("users").values([
      { id: 1, name: "Alice", email: "alice@example.com" },
      { id: 2, name: "Bob", email: "bob@example.com" },
      { id: 3, name: "Carol", email: "carol@example.com" },
    ]).execute();

    const result = await db.selectFrom("users").selectAll().where("id", ">", 1).execute();
    assert_equal(result.length, 2);
  });
});

test("kysely - select with and", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insertInto("users").values([
      { id: 1, name: "Alice", email: "alice@example.com", active: true },
      { id: 2, name: "Bob", email: "bob@example.com", active: false },
      { id: 3, name: "Carol", email: "carol@example.com", active: true },
    ]).execute();

    const result = await db
      .selectFrom("users")
      .selectAll()
      .where("active", "=", true)
      .where("id", ">", 1)
      .execute();
    assert_equal(result.length, 1);
    assert_equal(result[0].name, "Carol");
  });
});

test("kysely - select with or", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insertInto("users").values([
      { id: 1, name: "Alice", email: "alice@example.com" },
      { id: 2, name: "Bob", email: "bob@example.com" },
      { id: 3, name: "Carol", email: "carol@example.com" },
    ]).execute();

    const result = await db
      .selectFrom("users")
      .selectAll()
      .where((eb) => eb.or([eb("id", "=", 1), eb("id", "=", 3)]))
      .execute();
    assert_equal(result.length, 2);
  });
});

test("kysely - select specific columns", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insertInto("users").values({
      id: 1,
      name: "Alice",
      email: "alice@example.com",
    }).execute();

    const result = await db.selectFrom("users").select(["name", "email"]).execute();
    assert_equal(result.length, 1);
    assert_equal(result[0].name, "Alice");
    assert_equal(result[0].email, "alice@example.com");
    // @ts-expect-error id not selected
    assert_equal(result[0].id, undefined);
  });
});

test("kysely - select with order by asc", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insertInto("users").values([
      { id: 2, name: "Bob", email: "bob@example.com" },
      { id: 1, name: "Alice", email: "alice@example.com" },
      { id: 3, name: "Carol", email: "carol@example.com" },
    ]).execute();

    const result = await db.selectFrom("users").selectAll().orderBy("name", "asc").execute();
    assert_equal(result[0].name, "Alice");
    assert_equal(result[1].name, "Bob");
    assert_equal(result[2].name, "Carol");
  });
});

test("kysely - select with order by desc", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insertInto("users").values([
      { id: 1, name: "Alice", email: "alice@example.com" },
      { id: 2, name: "Bob", email: "bob@example.com" },
      { id: 3, name: "Carol", email: "carol@example.com" },
    ]).execute();

    const result = await db.selectFrom("users").selectAll().orderBy("id", "desc").execute();
    assert_equal(result[0].id, 3);
    assert_equal(result[1].id, 2);
    assert_equal(result[2].id, 1);
  });
});

test("kysely - select with limit", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insertInto("users").values([
      { id: 1, name: "Alice", email: "alice@example.com" },
      { id: 2, name: "Bob", email: "bob@example.com" },
      { id: 3, name: "Carol", email: "carol@example.com" },
    ]).execute();

    const result = await db
      .selectFrom("users")
      .selectAll()
      .orderBy("id", "asc")
      .limit(2)
      .execute();
    assert_equal(result.length, 2);
    assert_equal(result[0].id, 1);
    assert_equal(result[1].id, 2);
  });
});

test("kysely - select with offset", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insertInto("users").values([
      { id: 1, name: "Alice", email: "alice@example.com" },
      { id: 2, name: "Bob", email: "bob@example.com" },
      { id: 3, name: "Carol", email: "carol@example.com" },
    ]).execute();

    const result = await db
      .selectFrom("users")
      .selectAll()
      .orderBy("id", "asc")
      .limit(2)
      .offset(1)
      .execute();
    assert_equal(result.length, 2);
    assert_equal(result[0].id, 2);
    assert_equal(result[1].id, 3);
  });
});

// =============================================================================
// UPDATE Tests
// =============================================================================

test("kysely - update single field", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insertInto("users").values({
      id: 1,
      name: "Alice",
      email: "alice@example.com",
    }).execute();

    await db.updateTable("users").set({ name: "Alicia" }).where("id", "=", 1).execute();

    const result = await db.selectFrom("users").selectAll().where("id", "=", 1).execute();
    assert_equal(result[0].name, "Alicia");
  });
});

test("kysely - update multiple fields", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insertInto("users").values({
      id: 1,
      name: "Alice",
      email: "alice@example.com",
      active: true,
    }).execute();

    await db
      .updateTable("users")
      .set({ name: "Alicia", active: false })
      .where("id", "=", 1)
      .execute();

    const result = await db.selectFrom("users").selectAll().where("id", "=", 1).execute();
    assert_equal(result[0].name, "Alicia");
    assert_equal(result[0].active, false);
  });
});

test("kysely - update multiple rows", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insertInto("users").values([
      { id: 1, name: "Alice", email: "a@example.com", active: false },
      { id: 2, name: "Bob", email: "b@example.com", active: false },
      { id: 3, name: "Carol", email: "c@example.com", active: true },
    ]).execute();

    await db.updateTable("users").set({ active: true }).where("active", "=", false).execute();

    const result = await db.selectFrom("users").selectAll().execute();
    assert_equal(result.every((u) => u.active === true), true);
  });
});

// =============================================================================
// DELETE Tests
// =============================================================================

test("kysely - delete single row", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insertInto("users").values([
      { id: 1, name: "Alice", email: "alice@example.com" },
      { id: 2, name: "Bob", email: "bob@example.com" },
    ]).execute();

    await db.deleteFrom("users").where("id", "=", 1).execute();

    const result = await db.selectFrom("users").selectAll().execute();
    assert_equal(result.length, 1);
    assert_equal(result[0].name, "Bob");
  });
});

test("kysely - delete multiple rows", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insertInto("users").values([
      { id: 1, name: "Alice", email: "a@example.com", active: false },
      { id: 2, name: "Bob", email: "b@example.com", active: false },
      { id: 3, name: "Carol", email: "c@example.com", active: true },
    ]).execute();

    await db.deleteFrom("users").where("active", "=", false).execute();

    const result = await db.selectFrom("users").selectAll().execute();
    assert_equal(result.length, 1);
    assert_equal(result[0].name, "Carol");
  });
});

test("kysely - delete all rows", async () => {
  await withDb(async (db) => {
    await setupTables(db);
    await db.insertInto("users").values([
      { id: 1, name: "Alice", email: "alice@example.com" },
      { id: 2, name: "Bob", email: "bob@example.com" },
    ]).execute();

    await db.deleteFrom("users").execute();

    const result = await db.selectFrom("users").selectAll().execute();
    assert_equal(result.length, 0);
  });
});

// =============================================================================
// Multi-table Tests
// =============================================================================

test("kysely - insert and query related tables", async () => {
  await withDb(async (db) => {
    await setupTables(db);

    await db.insertInto("users").values({ id: 1, name: "Alice", email: "a@ex.com" }).execute();
    await db.insertInto("posts").values([
      { id: 1, title: "First Post", content: "Hello", author_id: 1 },
      { id: 2, title: "Second Post", content: "World", author_id: 1 },
    ]).execute();

    const userResult = await db.selectFrom("users").selectAll().execute();
    assert_equal(userResult.length, 1);

    const postResult = await db
      .selectFrom("posts")
      .selectAll()
      .where("author_id", "=", 1)
      .execute();
    assert_equal(postResult.length, 2);
  });
});

// kysely's `Migrator` over the stock `PostgresDialect` on the native server, unstubbed: the
// introspector's `getTables` query and the migration lock (`pg_advisory_xact_lock`, the
// writer here) run as kysely issues them. The npm leg's twin is
// `tests-web/npm/kysely_migrator_test.ts`.
test("kysely - Migrator migrates, reruns as a no-op, rolls a failing migration back, migrates down", async () => {
  const pool = new Pool(CONFIG);
  const db = new Kysely<any>({ dialect: new PostgresDialect({ pool }) });
  try {
    for (const t of ["kysely_migration", "kysely_migration_lock", "mig_person", "mig_pet"]) {
      await sql.raw(`DROP TABLE IF EXISTS ${t}`).execute(db);
    }
    const init: Migration = {
      up: async (db) => {
        await db.schema.createTable("mig_person").addColumn("id", "serial", (c) => c.primaryKey())
          .addColumn("name", "text").execute();
      },
      down: async (db) => {
        await db.schema.dropTable("mig_person").execute();
      },
    };
    const failing: Migration = {
      up: async (db) => {
        await db.schema.createTable("mig_pet").addColumn("id", "serial").execute();
        await sql`SELECT 1 / 0`.execute(db);
      },
    };
    let migrations: Record<string, Migration> = { "001_init": init };
    const migrator = new Migrator({ db, provider: { getMigrations: async () => migrations } });
    const names = async () =>
      (await db.introspection.getTables()).map((t) => t.name).filter((n) => n.startsWith("mig_")).sort();

    const first = await migrator.migrateToLatest();
    assert_equal(first.error, undefined);
    assert_equal(first.results?.map((r) => r.status), ["Success"]);
    assert_equal(await names(), ["mig_person"]);
    assert_equal((await migrator.migrateToLatest()).results, []);

    migrations = { "001_init": init, "002_failing": failing };
    const failed = await migrator.migrateToLatest();
    assert_equal((failed.error as { code?: string } | undefined)?.code, "22012");
    assert_equal(await names(), ["mig_person"]);

    migrations = { "001_init": init };
    const down = await migrator.migrateDown();
    assert_equal(down.error, undefined);
    assert_equal(await names(), []);
    for (const t of ["kysely_migration", "kysely_migration_lock"]) {
      await sql.raw(`DROP TABLE ${t}`).execute(db);
    }
  } finally {
    await db.destroy();
  }
});
