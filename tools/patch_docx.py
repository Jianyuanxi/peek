"""给 ui/vendor/docx-preview.min.js 打 Peek 补丁。升级该库后重跑；已打过的补丁会跳过。"""
import os, sys
P = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'ui', 'vendor', 'docx-preview.min.js')
s = open(P, encoding='utf8').read()
PATCHES = []

# 按字符数写的缩进（firstLineChars/hangingChars/leftChars/rightChars，中文 Word/WPS 最常见）：
# 非零时优先于 twips 值，单位是 1/100 字宽，用 em 近似；原库不认这些属性，编号段落的负首行缩进就伸出了表格边框
old = 'parseIndentation(e,t){var r=w.lengthAttr(e,"firstLine"),a=w.lengthAttr(e,"hanging"),s=w.lengthAttr(e,"left"),n=w.lengthAttr(e,"start"),l=w.lengthAttr(e,"right"),o=w.lengthAttr(e,"end");'
PATCHES.append(('Peek 补丁：*Chars', old, old + (
    '/* Peek 补丁：*Chars 缩进 */var C=function(k){var v=w.attr(e,k);return v&&0!=parseInt(v)?parseInt(v)/100+"em":null};'
    'r=C("firstLineChars")||r,a=C("hangingChars")||a,s=C("leftChars")||C("startChars")||s,l=C("rightChars")||C("endChars")||l;')))

# 分节符：原库只在显式分页符/lastRenderedPageBreak/纸张变化处分页。Word 保存时会写 lastRenderedPageBreak，
# WPS 不写，于是 WPS 文档的“下一页”分节全被并成一页。这里按下一节的 type（缺省 nextPage）在节末分页。
PATCHES.append(('Peek 补丁：分节分页',
    '(s||-1!=n)&&(r.sectProps=s,r.pageBreak=-1!=n,',
    # Word 会在新一节/新一页的第一段开头再写一个 lastRenderedPageBreak，刚分过页就不再分，免得多出空白页
    '(-1!=n&&0==l&&e.children.slice(0,n).every(c=>!c.children?.length)&&1==r.elements.length&&a.length>1&&(n=-1)),(s||-1!=n||(F=-1==n&&!!e.children?.some(c=>c.children?.some(d=>d.pageFill))))&&(r.sectProps=s,r.sectEnd=!!s,r.pageBreak=-1!=n||F,'))
PATCHES.append(('Peek 补丁：分节分页', 'var s=e.sectionProps,n=-1,l=-1;', 'var s=e.sectionProps,n=-1,l=-1,F=!1;'))
PATCHES.append(('Peek 补丁：分节分页',
    'for(let s of e)r.push(s),(this.options.ignoreLastRenderedPageBreak||s.pageBreak||this.isPageBreakSection(t,s.sectProps))&&a.push(r=[]),t=s.sectProps;',
    '/* Peek 补丁：分节分页 */for(let i=0;i<e.length;i++){let s=e[i];r.push(s),(this.options.ignoreLastRenderedPageBreak||s.pageBreak||this.isPageBreakSection(t,s.sectProps)'
    '||s.sectEnd&&e[i+1]&&"continuous"!=e[i+1].sectProps?.type)&&a.push(r=[]),t=s.sectProps}'))

# 四周型/紧密型环绕的锚定图：原库不认偏移，统统按行内块放在段首（WPS 封面整页图会挤在正文里、页眉横幅错位）。
# 改成左浮动 + 相对栏/段落的偏移；高到一页放不下正文的（整页封面）不占高度，并在所在段落后分页。
PATCHES.append(('Peek 补丁：环绕锚定图', 'case"wrapNone":a="wrapNone";break;',
    'case"wrapNone":a="wrapNone";break;case"wrapSquare":case"wrapTight":case"wrapThrough":a="wrap";break;'))
PATCHES.append(('Peek 补丁：环绕锚定图',
    '!r||"left"!=n.align&&"right"!=n.align||(t.cssStyle.float=n.align),t}',
    # n.align 缺省就是 "left"，所以先判环绕再判对齐，否则走不到偏移分支
    'r&&"wrap"==a&&!/^(right|center)$/.test(n.align)?(/* Peek 补丁：环绕锚定图 */t.cssStyle.float="left",'
    '/^(column|margin|character|leftMargin)$/.test(n.relative)&&n.offset&&(t.cssStyle["margin-left"]=n.offset),'
    '/^(paragraph|line)$/.test(l.relative)&&l.offset&&(t.cssStyle["margin-top"]=l.offset),'
    't.cssStyle["margin-right"]=w.lengthAttr(e,"distR",p)||"0",'
    'parseFloat(t.cssStyle.height)>=640&&(t.pageFill=!0,t.cssStyle["margin-bottom"]="-"+t.cssStyle.height)):r&&("left"==n.align||"right"==n.align)&&(t.cssStyle.float=n.align),t}'))

# 没写 headerReference/footerReference 的节按规范沿用上一节的页眉页脚，原库直接不画
PATCHES.append(('Peek 补丁：页眉沿用',
    'let o=a[r][0].sectProps;const i=this.createPageElement(',
    'let o=a[r][0].sectProps;/* Peek 补丁：页眉沿用（首页/偶数页/默认三种各自沿用） */var M=(p,c)=>[...(p||[]).filter(x=>!(c||[]).some(y=>y.type==x.type)),...(c||[])];'
    'o.headerRefs=M(this._ph,o.headerRefs),o.footerRefs=M(this._pf,o.footerRefs),this._ph=o.headerRefs,this._pf=o.footerRefs;const i=this.createPageElement('))

# 东亚字体：原库 parseFont 不认 eastAsiaTheme，主题也只导出拉丁字体，Word 默认样式（minorEastAsia）的中文落到浏览器默认黑体。
# 主题 <a:ea> 为空时 Word/WPS 用 <a:font script="Hans"> 指定的字体（一般是宋体/等线）
PATCHES.append(('Peek 补丁：东亚字体',
    'eaTypeface:t.elementAttr(e,"ea","typeface"),',
    'eaTypeface:t.elementAttr(e,"ea","typeface")||t.elements(e,"font").find(c=>"Hans"==t.attr(c,"script"))?.getAttribute("typeface"),'))
PATCHES.append(('Peek 补丁：东亚字体',
    'r.minorFont&&(t["--docx-minorHAnsi-font"]=r.minorFont.latinTypeface));',
    'r.minorFont&&(t["--docx-minorHAnsi-font"]=r.minorFont.latinTypeface),/* Peek 补丁：东亚字体 */'
    'r.majorFont?.eaTypeface&&(t["--docx-majorEastAsia-font"]=r.majorFont.eaTypeface),r.minorFont?.eaTypeface&&(t["--docx-minorEastAsia-font"]=r.minorFont.eaTypeface));'))
PATCHES.append(('Peek 补丁：东亚字体',
    'parseFont(e,t){var r=[w.attr(e,"ascii"),ze.themeValue(e,"asciiTheme"),w.attr(e,"eastAsia")]',
    'parseFont(e,t){var r=[w.attr(e,"ascii"),ze.themeValue(e,"asciiTheme"),w.attr(e,"eastAsia"),w.attr(e,"eastAsiaTheme")&&`var(--docx-${w.attr(e,"eastAsiaTheme")}-font, SimSun)`]'))

# 页码：原库把 PAGE 域的缓存结果原样画出，同一页脚部件在每页都一样（封面 I、摘要也是 I），也不认 pgNumType 的格式。
# 渲染时给 PAGE 域的结果打标记，每页画完页眉页脚后按节的 start（重新起编）/续编算页码，按 fmt 或域开关（\* ROMAN 等）格式化。
# 仍不按高度分页，长节只算一页，后面的页码会偏小。
PATCHES.append(('Peek 补丁：页码',
    'renderRun(e){if(e.fieldRun)return null;',
    'renderRun(e){if(e.fieldRun){/* Peek 补丁：页码 */for(const c of e.children)c.type==B.Instruction?this._fi=(this._fi||"")+c.text:c.type==B.ComplexField&&'
    '("begin"==c.charType?this._fi="":"separate"==c.charType?this._fp=/^\\s*PAGE\\b/.test(this._fi||"")&&(this._fn=(this._fn||0)+1)+"|"+this._fi:"end"==c.charType&&(this._fp=!1));return null}'))
PATCHES.append(('Peek 补丁：页码',
    'const r=this.toHTML(e,Ge.html,"span",t);return e.id&&(r.id=e.id),r}',
    'const r=this.toHTML(e,Ge.html,"span",t);return this._fp&&r.dataset&&(r.dataset.pg=this._fp),e.id&&(r.id=e.id),r}'))
PATCHES.append(('Peek 补丁：页码',
    'this.options.renderFooters&&this.renderHeaderFooter(o.footerRefs,o,t.length,s!=o,i),t.push(i),s=o}',
    'this.options.renderFooters&&this.renderHeaderFooter(o.footerRefs,o,t.length,s!=o,i),(()=>{/* Peek 补丁：页码 */const p=a[r][0].sectProps,q=p.pageNumber;'
    'this._pg=s!=p&&null!=q?.start?q.start:(this._pg||0)+1;const n=this._pg,R=()=>{let m=n,v="";for(const[k,c]of[[1e3,"M"],[900,"CM"],[500,"D"],[400,"CD"],[100,"C"],[90,"XC"],[50,"L"],[40,"XL"],[10,"X"],[9,"IX"],[5,"V"],[4,"IV"],[1,"I"]])for(;m>=k;m-=k)v+=c;return v},'
    'L=()=>String.fromCharCode(65+(n-1)%26).repeat(Math.floor((n-1)/26)+1),seen={};'
    'i.querySelectorAll("[data-pg]").forEach(x=>{const[id,ins]=x.dataset.pg.split("|");if(seen[id])return void(x.textContent="");seen[id]=1;'
    'const sw=/\\\\\\*\\s*(\\w+)/.exec(ins)?.[1],f={ROMAN:"upperRoman",roman:"lowerRoman",Roman:"upperRoman",ALPHABETIC:"upperLetter",alphabetic:"lowerLetter",Arabic:"decimal"}[sw]||q?.format||"decimal",'
    'v={decimal:n,upperRoman:R(),lowerRoman:R().toLowerCase(),upperLetter:L(),lowerLetter:L().toLowerCase(),numberInDash:"- "+n+" -"}[f];null!=v&&(x.textContent=String(v))})})(),t.push(i),s=o}'))
# 设了“首页不同”（titlePg）但这一节和前面都没有首页页眉时，首页页眉为空；原库会退回默认页眉，封面多出页眉
PATCHES.append(('Peek 补丁：首页页眉',
    'var l=(t.titlePage&&a?e.find(e=>"first"==e.type):null)??(r%2==1&&n?e.find(e=>"even"==e.type):null)??e.find(e=>"default"==e.type)',
    'var l=t.titlePage&&a?e.find(e=>"first"==e.type):(r%2==1&&n?e.find(e=>"even"==e.type):null)??e.find(e=>"default"==e.type)'))

# 制表位：位置从版心左边算起。原库用段落盒子左边（已含左缩进）再加一次左缩进当原点，缩进段落（目录二级项）的第一个制表位跳得太远
PATCHES.append(('Peek 补丁：制表位原点',
    'const p=parseFloat(o.marginLeft),d=l.left+p,',
    'const p=parseFloat(o.marginLeft),d=l.left-p/* Peek 补丁：制表位原点 */,'))

done = 0
for mark, old, new in PATCHES:
    if new in s:
        continue
    assert s.count(old) == 1, (mark, s.count(old))
    s = s.replace(old, new)
    done += 1
open(P, 'w', encoding='utf8', newline='').write(s)
print('patched', done)
