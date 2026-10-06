/**
 * 生成 Office→PDF 保真度测试样本。
 * 用法：node scripts/make-fixtures.mjs [输出目录]
 *
 * 手写最小可用的 OOXML 打包成 ZIP（stored，不压缩）。
 * 用 Node 而不是 PowerShell 是因为样本里必须有中文，
 * 而 Windows PowerShell 5.1 读 .ps1 用的是系统 ANSI 码页，中文会被破坏。
 */
import { writeFileSync, mkdirSync, existsSync } from 'node:fs';
import { resolve } from 'node:path';

const CRC_TABLE = (() => {
  const t = new Int32Array(256);
  for (let n = 0; n < 256; n++) {
    let c = n;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[n] = c;
  }
  return t;
})();

function crc32(buf) {
  let c = -1;
  for (let i = 0; i < buf.length; i++) c = CRC_TABLE[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  return (c ^ -1) >>> 0;
}

/** 极简 ZIP 写入器（全部 stored，无压缩）。文件都很小，不值得做 deflate。 */
function makeZip(entries) {
  const locals = [];
  const centrals = [];
  let offset = 0;

  for (const [name, content] of entries) {
    const nameBuf = Buffer.from(name, 'utf8');
    const data = Buffer.from(content, 'utf8');
    const crc = crc32(data);

    const local = Buffer.alloc(30 + nameBuf.length);
    local.writeUInt32LE(0x04034b50, 0);   // 本地文件头签名
    local.writeUInt16LE(20, 4);           // 解压所需版本
    local.writeUInt16LE(0x0800, 6);       // 通用标志位：文件名为 UTF-8
    local.writeUInt16LE(0, 8);            // 压缩方式：stored
    local.writeUInt16LE(0, 10);           // 修改时间
    local.writeUInt16LE(0x21, 12);        // 修改日期（1980-01-01）
    local.writeUInt32LE(crc, 14);
    local.writeUInt32LE(data.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(nameBuf.length, 26);
    local.writeUInt16LE(0, 28);           // 扩展字段长度
    nameBuf.copy(local, 30);

    locals.push(local, data);

    const central = Buffer.alloc(46 + nameBuf.length);
    central.writeUInt32LE(0x02014b50, 0); // 中央目录签名
    central.writeUInt16LE(20, 4);         // 创建版本
    central.writeUInt16LE(20, 6);         // 解压所需版本
    central.writeUInt16LE(0x0800, 8);
    central.writeUInt16LE(0, 10);
    central.writeUInt16LE(0, 12);
    central.writeUInt16LE(0x21, 14);
    central.writeUInt32LE(crc, 16);
    central.writeUInt32LE(data.length, 20);
    central.writeUInt32LE(data.length, 24);
    central.writeUInt16LE(nameBuf.length, 28);
    central.writeUInt16LE(0, 30);
    central.writeUInt16LE(0, 32);
    central.writeUInt16LE(0, 34);
    central.writeUInt16LE(0, 36);
    central.writeUInt32LE(0, 38);
    central.writeUInt32LE(offset, 42);
    nameBuf.copy(central, 46);

    centrals.push(central);
    offset += local.length + data.length;
  }

  const centralBuf = Buffer.concat(centrals);
  const eocd = Buffer.alloc(22);
  eocd.writeUInt32LE(0x06054b50, 0);
  eocd.writeUInt16LE(0, 4);
  eocd.writeUInt16LE(0, 6);
  eocd.writeUInt16LE(centrals.length, 8);
  eocd.writeUInt16LE(centrals.length, 10);
  eocd.writeUInt32LE(centralBuf.length, 12);
  eocd.writeUInt32LE(offset, 16);
  eocd.writeUInt16LE(0, 20);

  return Buffer.concat([...locals, centralBuf, eocd]);
}

/* ---------------------------------------------------------------- DOCX */

const W = 'http://schemas.openxmlformats.org/wordprocessingml/2006/main';

const docxContentTypes = `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
</Types>`;

const docxRels = `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>`;

function cell(text, bold = false) {
  return `<w:tc><w:tcPr><w:tcW w:w="2400" w:type="dxa"/></w:tcPr><w:p><w:r>${
    bold ? '<w:rPr><w:b/></w:rPr>' : ''
  }<w:t xml:space="preserve">${text}</w:t></w:r></w:p></w:tc>`;
}

function row(cells) {
  return `<w:tr>${cells}</w:tr>`;
}

const tblBorders = `<w:tblPr><w:tblW w:w="0" w:type="auto"/><w:tblBorders>
  <w:top w:val="single" w:sz="8" w:color="4472C4"/>
  <w:left w:val="single" w:sz="8" w:color="4472C4"/>
  <w:bottom w:val="single" w:sz="8" w:color="4472C4"/>
  <w:right w:val="single" w:sz="8" w:color="4472C4"/>
  <w:insideH w:val="single" w:sz="4" w:color="AAAAAA"/>
  <w:insideV w:val="single" w:sz="4" w:color="AAAAAA"/>
</w:tblBorders></w:tblPr>`;

const docxDocument = `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="${W}">
  <w:body>
    <w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:rPr><w:b/><w:sz w:val="40"/></w:rPr><w:t>格式转换保真度测试</w:t></w:r></w:p>

    <w:p><w:pPr><w:spacing w:before="240" w:after="120"/></w:pPr><w:r><w:rPr><w:b/><w:sz w:val="28"/></w:rPr><w:t>一、中文混排与标点</w:t></w:r></w:p>

    <w:p><w:r><w:t xml:space="preserve">这是一段用于检验字体回退与断行位置的中文正文。标点符号包括：逗号，句号。顿号、分号；问号？感叹号！引号“双引号”与‘单引号’，以及括号（全角）和省略号……</w:t></w:r></w:p>

    <w:p><w:r><w:t xml:space="preserve">Mixed Latin text with Chinese 混排 words, numbers 1234567890, and symbols !@#$%^&amp;*() to check baseline alignment and kerning across font fallback boundaries.</w:t></w:r></w:p>

    <w:p><w:r><w:t xml:space="preserve">超长行不留手写换行，用于检验自动断行的位置是否与 Word 完全一致。段落宽度固定，若断行点不同则说明字体度量或排版引擎存在差异，这类差异在正式文档里最容易被发现。再补一些字确保换到第三行上去。</w:t></w:r></w:p>

    <w:p><w:pPr><w:spacing w:before="240" w:after="120"/></w:pPr><w:r><w:rPr><w:b/><w:sz w:val="28"/></w:rPr><w:t>二、表格与合并单元格</w:t></w:r></w:p>

    <w:tbl>${tblBorders}
      ${row(cell('项目', true) + cell('数值', true) + cell('备注', true))}
      ${row(cell('分辨率') + cell('3840×2160') + cell('4K UHD'))}
      ${row(cell('色深') + cell('10 bit') + cell('HDR10'))}
      ${row(cell('码率') + cell('48 Mbps') + cell('可变'))}
    </w:tbl>

    <w:p><w:pPr><w:spacing w:before="240" w:after="120"/></w:pPr><w:r><w:rPr><w:b/><w:sz w:val="28"/></w:rPr><w:t>三、列表与对齐</w:t></w:r></w:p>

    <w:p><w:pPr><w:jc w:val="left"/></w:pPr><w:r><w:t>· 左对齐条目</w:t></w:r></w:p>
    <w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:t>· 居中对齐条目</w:t></w:r></w:p>
    <w:p><w:pPr><w:jc w:val="right"/></w:pPr><w:r><w:t>· 右对齐条目</w:t></w:r></w:p>

    <w:p><w:r><w:br w:type="page"/></w:r></w:p>

    <w:p><w:pPr><w:jc w:val="center"/></w:pPr><w:r><w:rPr><w:b/><w:sz w:val="32"/></w:rPr><w:t>第二页 · 分页位置检验</w:t></w:r></w:p>

    <w:p><w:r><w:t xml:space="preserve">如果这一页出现在 PDF 的第二页，说明分页行为与 Word 一致。反之则说明分页被重新计算过。</w:t></w:r></w:p>

    <w:sectPr>
      <w:pgSz w:w="11906" w:h="16838"/>
      <w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="851" w:footer="992" w:gutter="0"/>
    </w:sectPr>
  </w:body>
</w:document>`;

/* ---------------------------------------------------------------- XLSX */

const xlsxContentTypes = `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
  <Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
</Types>`;

const xlsxRels = `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>`;

const xlsxWorkbook = `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <sheets><sheet name="销售数据" sheetId="1" r:id="rId1"/></sheets>
</workbook>`;

const xlsxWorkbookRels = `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
</Relationships>`;

function sCell(ref, text, isNum = false) {
  const t = isNum ? 'n' : 'inlineStr';
  const inner = isNum ? `<v>${text}</v>` : `<is><t>${text}</t></is>`;
  return `<c r="${ref}" t="${t}">${inner}</c>`;
}

const xlsxSheet = `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
  <sheetData>
    <row r="1">${sCell('A1', '地区')}${sCell('B1', '销量')}${sCell('C1', '金额')}</row>
    <row r="2">${sCell('A2', '华东')}${sCell('B2', '1280', true)}${sCell('C2', '38400.5', true)}</row>
    <row r="3">${sCell('A3', '华北')}${sCell('B3', '960', true)}${sCell('C3', '28800', true)}</row>
    <row r="4">${sCell('A4', '华南')}${sCell('B4', '1520', true)}${sCell('C4', '45600.75', true)}</row>
  </sheetData>
</worksheet>`;

/* ---------------------------------------------------------------- 输出 */

const outDir = resolve(process.argv[2] || './fixtures');
if (!existsSync(outDir)) mkdirSync(outDir, { recursive: true });

writeFileSync(resolve(outDir, '保真度测试.docx'), makeZip([
  ['[Content_Types].xml', docxContentTypes],
  ['_rels/.rels', docxRels],
  ['word/document.xml', docxDocument],
]));
console.log('  保真度测试.docx');

writeFileSync(resolve(outDir, '销售数据.xlsx'), makeZip([
  ['[Content_Types].xml', xlsxContentTypes],
  ['_rels/.rels', xlsxRels],
  ['xl/workbook.xml', xlsxWorkbook],
  ['xl/_rels/workbook.xml.rels', xlsxWorkbookRels],
  ['xl/worksheets/sheet1.xml', xlsxSheet],
]));
console.log('  销售数据.xlsx');
