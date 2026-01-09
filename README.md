# CSV Translator MCP Server

MCP server bằng Rust để dịch file CSV lớn - **AI tự dịch trong context** (không cần API key riêng).

## Tính năng

- **AI Self-Translation**: AI tự dịch từng chunk, không cần gọi API bên ngoài
- **Local Mode**: Đọc file trực tiếp từ đường dẫn local
- **Memory-efficient**: Streaming xử lý file lớn
- **Context-aware**: Overlap rows giúp AI hiểu ngữ cảnh
- **Chunk-by-chunk**: Xử lý từng phần nhỏ, tránh phình context

## Cài đặt

```bash
cd /home/minhhieu/projects/mcp-csv-transalate
cargo build --release
```

## Cấu hình với Amp

Thêm vào `~/.config/amp/settings.json`:

```json
{
  "mcpServers": {
    "csv-translator": {
      "command": "/path/to/csv-translator-mcp"
    }
  }
}
```

## Workflow (AI Self-Translation)

### Bước 1: Phân tích file
```
analyze_csv(file_path="/path/to/data.csv")
```

### Bước 2: Cấu hình
```
configure_translation(
  source_lang="vi",
  target_lang="en",
  columns_to_translate=["name", "description"],
  chunk_size=50,
  domain_context="e-commerce"
)
```

### Bước 3: Khởi tạo session
```
init_translation(
  input_file="/path/to/input.csv",
  output_file="/path/to/output.csv"
)
→ session_id="abc-123"
```

### Bước 4: Lấy chunk và dịch (lặp lại)
```
get_next_chunk(session_id="abc-123")
→ Trả về rows cần dịch với instructions

[AI tự dịch các core rows]

submit_translation(
  session_id="abc-123",
  chunk_index=0,
  translated_rows=[["translated_name", "translated_desc"], ...]
)
```

### Bước 5: Lặp lại bước 4 đến khi hoàn thành

## MCP Tools

| Tool | Mô tả |
|------|-------|
| `analyze_csv` | Phân tích metadata file CSV |
| `configure_translation` | Cấu hình ngôn ngữ, cột dịch, domain |
| `init_translation` | Khởi tạo session dịch |
| `get_next_chunk` | Lấy chunk tiếp theo cần dịch |
| `submit_translation` | Gửi bản dịch của chunk |
| `get_translation_progress` | Xem tiến độ |

## Ví dụ sử dụng với AI

```
User: Dịch file products.csv từ tiếng Việt sang tiếng Anh

AI: 
1. Gọi analyze_csv để xem cấu trúc file
2. Gọi configure_translation với source=vi, target=en
3. Gọi init_translation để bắt đầu
4. Lặp:
   - Gọi get_next_chunk để lấy dữ liệu
   - Tự dịch các core rows
   - Gọi submit_translation để lưu
5. Hoàn thành khi get_next_chunk trả về status=completed
```

## Cấu trúc Chunk

Mỗi chunk có:
- **Overlap rows đầu**: Ngữ cảnh từ chunk trước
- **Core rows**: Phần cần dịch và trả về
- **Overlap rows cuối**: Ngữ cảnh cho câu tiếp

AI chỉ dịch và submit **core rows**, overlap chỉ để hiểu ngữ cảnh.

## License

MIT
