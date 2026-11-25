// Minimal JSON-lines adapter. Each build links one native engine, no Python runtime.
#include <chrono>
#include <iostream>
#include <stdexcept>
#include <string>
#include "json.hpp"
#if defined(ENGINE_SQLITE)
#include <sqlite3.h>
#include "sqlite-vec.h"
#elif defined(ENGINE_COZO)
#include "cozo_c.h"
#else
#include "lbug.h"
#endif

using Json = nlohmann::json;
using Clock = std::chrono::steady_clock;

#if defined(ENGINE_SQLITE)
struct Engine {
    sqlite3* db = nullptr;
    explicit Engine(const char* path) {
        if (sqlite3_open(path, &db) != SQLITE_OK) throw std::runtime_error("sqlite open failed");
        char* error = nullptr;
        if (sqlite3_vec_init(db, &error, nullptr) != SQLITE_OK) {
            std::string message = error ? error : "vec init failed";
            sqlite3_free(error);
            throw std::runtime_error(message);
        }
    }
    ~Engine() { sqlite3_close(db); }
    Json run(const Json& command) {
        sqlite3_stmt* statement = nullptr;
        std::string sql = command.at("sql");
        if (sqlite3_prepare_v2(db, sql.c_str(), -1, &statement, nullptr) != SQLITE_OK)
            throw std::runtime_error(sqlite3_errmsg(db));
        struct Finalize { sqlite3_stmt* s; ~Finalize() { sqlite3_finalize(s); } } cleanup{statement};
        Json params = command.value("params", Json::array());
        for (size_t i = 0; i < params.size(); ++i) {
            const auto& p = params[i];
            if (p.is_null()) sqlite3_bind_null(statement, i + 1);
            else if (p.is_number_integer()) sqlite3_bind_int64(statement, i + 1, p.get<int64_t>());
            else if (p.is_number()) sqlite3_bind_double(statement, i + 1, p.get<double>());
            else {
                std::string value = p.is_string() ? p.get<std::string>() : p.dump();
                sqlite3_bind_text(statement, i + 1, value.c_str(), -1, SQLITE_TRANSIENT);
            }
        }
        Json rows = Json::array();
        int status;
        while ((status = sqlite3_step(statement)) == SQLITE_ROW) {
            Json row = Json::array();
            for (int i = 0; i < sqlite3_column_count(statement); ++i) {
                switch (sqlite3_column_type(statement, i)) {
                    case SQLITE_INTEGER: row.push_back(sqlite3_column_int64(statement, i)); break;
                    case SQLITE_FLOAT: row.push_back(sqlite3_column_double(statement, i)); break;
                    case SQLITE_NULL: row.push_back(nullptr); break;
                    default: row.push_back(reinterpret_cast<const char*>(sqlite3_column_text(statement, i)));
                }
            }
            rows.push_back(row);
        }
        if (status != SQLITE_DONE) throw std::runtime_error(sqlite3_errmsg(db));
        return rows;
    }
};
#elif defined(ENGINE_COZO)
struct Engine {
    int32_t db;
    explicit Engine(const char* path) {
        char* error = cozo_open_db("sqlite", path, "{}", &db);
        if (error) { std::string message(error); cozo_free_str(error); throw std::runtime_error(message); }
    }
    ~Engine() { cozo_close_db(db); }
    Json run(const Json& command) {
        std::string sql = command.at("sql");
        std::string params = command.value("params", Json::object()).dump();
        char* result = cozo_run_query(db, sql.c_str(), params.c_str(), false);
        Json parsed;
        try { parsed = Json::parse(result); } catch (...) { cozo_free_str(result); throw; }
        cozo_free_str(result);
        if (!parsed.value("ok", false)) throw std::runtime_error(parsed.dump());
        return parsed.value("rows", Json::array());
    }
};
#else
struct Engine {
    lbug_database db{};
    lbug_connection connection{};
    explicit Engine(const char* path) {
        auto config = lbug_default_system_config();
        config.buffer_pool_size = 128 * 1024 * 1024;
        config.max_num_threads = 2;
        if (lbug_database_init(path, config, &db) != LbugSuccess)
            throw std::runtime_error("ladybug open failed");
        if (lbug_connection_init(&db, &connection) != LbugSuccess)
            throw std::runtime_error("ladybug connection failed");
    }
    ~Engine() { lbug_connection_destroy(&connection); lbug_database_destroy(&db); }
    Json run(const Json& command) {
        lbug_query_result result{};
        std::string sql = command.at("sql");
        lbug_connection_query(&connection, sql.c_str(), &result);
        struct Cleanup { lbug_query_result* r; ~Cleanup() { lbug_query_result_destroy(r); } } cleanup{&result};
        if (!lbug_query_result_is_success(&result)) {
            char* raw = lbug_query_result_get_error_message(&result);
            std::string error = raw ? raw : "ladybug query failed";
            lbug_destroy_string(raw);
            throw std::runtime_error(error);
        }
        Json rows = Json::array();
        while (lbug_query_result_has_next(&result)) {
            lbug_flat_tuple tuple{};
            lbug_query_result_get_next(&result, &tuple);
            Json row = Json::array();
            for (uint64_t i = 0; i < lbug_query_result_get_num_columns(&result); ++i) {
                lbug_value value{};
                lbug_flat_tuple_get_value(&tuple, i, &value);
                if (lbug_value_is_null(&value)) row.push_back(nullptr);
                else {
                    lbug_logical_type type{};
                    lbug_value_get_data_type(&value, &type);
                    if (lbug_data_type_get_id(&type) == LBUG_STRING) {
                        char* raw = nullptr;
                        lbug_value_get_string(&value, &raw);
                        row.push_back(raw ? raw : "");
                        lbug_destroy_string(raw);
                    } else {
                        char* raw = lbug_value_to_string(&value);
                        auto parsed = Json::parse(raw ? raw : "null", nullptr, false);
                        row.push_back(parsed.is_discarded() ? Json(raw ? raw : "") : parsed);
                        lbug_destroy_string(raw);
                    }
                    lbug_data_type_destroy(&type);
                }
                lbug_value_destroy(&value);
            }
            lbug_flat_tuple_destroy(&tuple);
            rows.push_back(row);
        }
        return rows;
    }
};
#endif

int main(int argc, char** argv) {
    try {
        if (argc != 2) throw std::runtime_error("expected database path");
        Engine engine(argv[1]);
        std::string line;
        while (std::getline(std::cin, line)) {
            auto started = Clock::now();
            Json response;
            try { response = {{"ok", true}, {"rows", engine.run(Json::parse(line))}}; }
            catch (const std::exception& error) { response = {{"ok", false}, {"error", error.what()}}; }
            response["seconds"] = std::chrono::duration<double>(Clock::now() - started).count();
            std::cout << response.dump() << std::endl;
        }
    } catch (const std::exception& error) {
        std::cerr << error.what() << std::endl;
        return 1;
    }
}
