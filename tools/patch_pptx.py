"""给 ui/vendor/pptx-preview.umd.js 打 Peek 补丁（第 1–12 项）。
基准是只带 rot/flip 补丁的版本：git show 8519eee:ui/vendor/pptx-preview.umd.js > ui/vendor/pptx-preview.umd.js；
升级该库时先手工补回 rot/flip，再跑本脚本。每项替换都核对命中次数，对不上就报错退出。"""
import os, sys
P = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'ui', 'vendor', 'pptx-preview.umd.js')
s = open(P, encoding='utf8').read()
if 'Peek 补丁：跳过不存在的部件' in s:
    sys.exit('已经打过补丁；要重打先 git checkout 该文件')


def rep(old, new, count=1):
    global s
    n = s.count(old)
    if n != count:
        sys.exit(f'expected {count} got {n}: {old[:80]}')
    s = s.replace(old, new)


# 1. [Content_Types].xml 里登记了但包里不存在的部件（如多余的 slideMaster2..19）直接跳过；原库读到就抛错，整份演示文稿一页都不出
rep('n=t.Types.Override,',
    'n=[].concat(t.Types.Override||[]).filter((function(t){return!!this._zipContents.files[t.attrs.PartName.substr(1)]}).bind(this))/* Peek 补丁：跳过不存在的部件 */,')

# 2. 单个版式/幻灯片解析出错只影响它自己；原库所有页共用一个 try，一页出错后面全部丢掉
rep('[4,(g=new Rr(m,_,this)).load()]',
    '[4,(g=new Rr(m,_,this)).load().catch((function(e){console.warn("pptx layout",m,e)}))/* Peek 补丁 */]')
rep('[4,(b=new Pr(m,_,this)).load()]',
    '[4,(b=new Pr(m,_,this)).load().catch((function(e){console.warn("pptx slide",m,e)}))/* Peek 补丁 */]')

# 3. 图片引用了不存在的关系 ID（链接图片、损坏的引用）就跳过这张图，不让整页报错
rep('p=d[I],f=p["p:blipFill"]["a:blip"].attrs["r:embed"],g=r.rels[f].target,y=new Mr(g,p,n,r,i),t.push(y);',
    'p=d[I],f=Yn.get(p,["p:blipFill","a:blip","attrs","r:embed"])||function(x){x=[].concat(Yn.get(x,["a:extLst","a:ext"])||[]);for(var k=0;k<x.length;k++){var v=Yn.get(x[k],["asvg:svgBlip","attrs","r:embed"]);if(v)return v}}(Yn.get(p,["p:blipFill","a:blip"])),'
    'r.rels[f]&&(g=r.rels[f].target,y=new Mr(g,p,n,r,i),t.push(y))/* Peek 补丁：缺图跳过；只有 SVG 没有 PNG 备份的用 SVG */;')

# 4. 幻灯片按 presentation.xml 的 sldIdLst 排序（原库按文件名 slideN 的数字排，和 PowerPoint/WPS 的页序不一致），不在列表里的孤立页不显示
rep('case 4:return r.sent(),[4,this._loadContentTypes()];case 5:return r.sent(),[2]',
    'case 4:return r.sent(),[4,this._loadContentTypes()];case 5:return r.sent(),[4,this.__peekOrder()];case 6:return r.sent(),[2]')
ORDER = ('t.prototype.__peekOrder=function(){/* Peek 补丁：按 sldIdLst 排页 */var s=this,f=s._zipContents.files,a=f["ppt/presentation.xml"],b=f["ppt/_rels/presentation.xml.rels"];'
         'if(!a||!b)return Promise.resolve();return Promise.all([a.async("text"),b.async("text")]).then((function(x){'
         'var rel={},m,re=/<Relationship\\b[^>]*>/g;while(m=re.exec(x[1])){var id=/\\bId="([^"]+)"/.exec(m[0]),tg=/\\bTarget="([^"]+)"/.exec(m[0]);'
         'if(id&&tg)rel[id[1]]=tg[1].charAt(0)==="/"?tg[1].substr(1):"ppt/"+tg[1].replace(/^\\.\\//,"")}'
         'var ord={},i=0,r2=/<p:sldId\\b[^>]*\\br:id="([^"]+)"/g;while(m=r2.exec(x[0]))rel[m[1]]&&!(rel[m[1]] in ord)&&(ord[rel[m[1]]]=i++);'
         'i&&(s.slides=s.slides.filter((function(z){return z.name in ord})).sort((function(p,q){return ord[p.name]-ord[q.name]})))'
         '})).catch((function(e){console.warn("pptx order",e)}))},')
rep('t.prototype._loadThumbnail=function(){', ORDER + 't.prototype._loadThumbnail=function(){')

# 5. 形状写了 <a:ln><a:noFill/></a:ln>（无边框）时不再套用主题 lnRef 的边框；原库 wr() 对 noFill 返回 undefined，Object.assign 什么也不做
rep('var i=Yn.get(this.source,["p:spPr","a:ln"]);i&&Object.assign(this.border,wr(i,this.theme,this))',
    'var i=Yn.get(this.source,["p:spPr","a:ln"]);i&&(Yn.get(i,"a:noFill")?this.border={}/* Peek 补丁：无边框 */:Object.assign(this.border,wr(i,this.theme,this)))')

# 6. 母版/版式里所有非占位符形状都要画（PowerPoint/WPS 的行为）；原库只画带 userDrawn="1" 的，装饰条、线条、色块大多丢失
rep('var a=Yn.get(o,"p:ph");a&&a.attrs&&(this.idx=a.attrs.idx,this.type=a.attrs.type)',
    'var a=Yn.get(o,"p:ph");a&&(this.isPh=!0),a&&a.attrs&&(this.idx=a.attrs.idx,this.type=a.attrs.type)')
rep('var r=c([],t.nodes,!0).filter((function(t){return t.userDrawn}));',
    'var r=c([],t.nodes,!0).filter((function(t){return t.userDrawn||!t.isPh}))/* Peek 补丁 */;', count=2)

# 7. 幻灯片/版式 showMasterSp="0" 时不画母版（和版式）上的图形；母版或版式缺失时跳过而不是报错
rep('this._renderSlideMaster(e.slideMaster,n),this._renderSlideLayout(e.slideLayout,n),',
    '(function(x){var r=x&&x.source&&(x.source["p:sld"]||x.source["p:sldLayout"]);return!(r&&r.attrs&&"0"===r.attrs.showMasterSp)})/* Peek 补丁 */.call(null,e)'
    '&&(e.slideMaster&&(function(x){var r=x&&x.source&&x.source["p:sldLayout"];return!(r&&r.attrs&&"0"===r.attrs.showMasterSp)})(e.slideLayout)&&this._renderSlideMaster(e.slideMaster,n),e.slideLayout&&this._renderSlideLayout(e.slideLayout,n)),')

# 8. 段落对齐：补上两端对齐 just（中文文本框最常见），没写对齐时按 PowerPoint 默认左对齐；原库都当居中
rep('v.style.textAlign=d.align&&{ctr:"center",l:"left",r:"right",dist:"justify"}[d.align]||"center"',
    'v.style.textAlign=d.align&&{ctr:"center",l:"left",r:"right",dist:"justify",just:"justify",justLow:"justify",thaiDist:"justify"}[d.align]||"left"/* Peek 补丁 */')

# 9. 预设颜色 prstClr 上的亮度/透明度修饰（lumMod/lumOff 等）也要生效，原库只认 srgb/scheme/sys
rep('var a=t["a:srgbClr"]||t["a:schemeClr"]||t["a:sysClr"],',
    'var a=t["a:srgbClr"]||t["a:schemeClr"]||t["a:sysClr"]||t["a:prstClr"]/* Peek 补丁 */,')

# 10. 正文占位符继承母版 bodyStyle（项目符号、字体、字号）；原库只给标题套 titleStyle，正文占位符的 bodyStyle 解析了却从没用上
for k in ('props', 'defRPr'):
    rep('["subTitle","ctrTitle","title"].includes(i)&&Object.assign(n,Yn.get(r.slideLayout.slideMaster,["textStyles","titleStyle","lvl".concat(t?+t+1:1),"%s"])||{})' % k,
        '(["ctrTitle","title"].includes(i)?Object.assign(n,Yn.get(r.slideLayout.slideMaster,["textStyles","titleStyle","lvl".concat(t?+t+1:1),"%s"])||{})'
        ':["dt","ftr","sldNum","hdr"].includes(i)||Object.assign(n,Yn.get(r.slideLayout.slideMaster,["textStyles","bodyStyle","lvl".concat(t?+t+1:1),"%s"])||{}))/* Peek 补丁 */' % (k, k))

# 11. 形状自己 <a:lstStyle> 里的段落属性（项目符号、缩进、对齐）也要继承；原库只对字符属性 defRPr 用了它
rep('}return n},t.prototype._getInheritRProps',
    '}return Yn.get(this.lstStyle,["lvl".concat(t?+t+1:1),"props"])&&Object.assign(n,Yn.get(this.lstStyle,["lvl".concat(t?+t+1:1),"props"]))/* Peek 补丁 */,n},t.prototype._getInheritRProps')

# 12. 项目符号的大小（buSzPct）和颜色（buClr）；原库符号永远和第一个字一样大、一样色
rep('Yn.get(t,["a:buChar","attrs","char"])&&(e.buChar=t["a:buChar"].attrs.char),',
    'Yn.get(t,["a:buChar","attrs","char"])&&(e.buChar=t["a:buChar"].attrs.char),'
    'Yn.get(t,["a:buSzPct","attrs","val"])&&(e.buSzPct=parseInt(t["a:buSzPct"].attrs.val)/1e5),'
    'function(s){try{t["a:buClr"]&&s.node&&s.node.theme&&(e.buClr=dr(ir(t["a:buClr"],s.node.theme,s.node)))}catch(x){}}(this)/* Peek 补丁 */,', count=2)
rep('n.style.color=r.style.color,n.style.fontSize=r.style.fontSize,n.textContent={n:"■"',
    'n.style.color=e.buClr||r.style.color,n.style.fontSize=parseFloat(r.style.fontSize)*(e.buSzPct||1)+"px",n.style.verticalAlign="middle"/* Peek 补丁 */,n.textContent={n:"■"')

# 13. 柱状/折线图按图表 XML 调整 echarts 配置：坐标轴 min/max/majorUnit、Excel 式非零起点、网格线、
#     每个数据点自己的颜色（dPt）和 varyColors、图例有无与位置、标题（无标题时不显示“图表标题”，不默认加粗）
CHART = ('e.prototype.__peekChart=function(t,e){/* Peek 补丁：图表细节 */try{var s=this,o=this.options,ser=Yn.get(t,["c:ser"]);Array.isArray(ser)||(ser=[ser]);'
         'var cl=function(x){try{return x?dr(ir(x,s.theme,s.ctx)):void 0}catch(r){}};'
         'var hz=o.yAxis&&"category"===o.yAxis.type,va=hz?o.xAxis:o.yAxis,vx=Yn.get(e,["c:plotArea","c:valAx"]);Array.isArray(vx)&&(vx=vx[0]);'
         'var mn=Yn.get(vx,["c:scaling","c:min","attrs","val"]),mx=Yn.get(vx,["c:scaling","c:max","attrs","val"]),mu=Yn.get(vx,["c:majorUnit","attrs","val"]);'
         'null!=mn&&(va.min=+mn),null!=mx&&(va.max=+mx),null!=mu&&(va.interval=+mu);'
         'if(null==mn&&!o.series.some((function(x){return x.stack}))){var al=[];o.series.forEach((function(x){al=al.concat(x.data.filter((function(v){return isFinite(v)})))}));'
         'var lo=Math.min.apply(null,al),hi=Math.max.apply(null,al);al.length&&lo>0&&lo>5*hi/6&&(va.scale=!0)}'
         'vx&&(va.splitLine={show:!!Yn.get(vx,["c:majorGridlines"])},"1"===Yn.get(vx,["c:delete","attrs","val"])&&(va.show=!1));'
         'var vary="1"===Yn.get(t,["c:varyColors","attrs","val"])&&1===ser.length,acc=["accent1","accent2","accent3","accent4","accent5","accent6"];'
         'ser.forEach((function(x,i){var d=Yn.get(x,["c:dPt"])||[],c={};Array.isArray(d)||(d=[d]),d.forEach((function(p){var k=Yn.get(p,["c:idx","attrs","val"]),v=cl(Yn.get(p,["c:spPr","a:solidFill"]));null!=k&&v&&(c[k]=v)}));'
         'o.series[i]&&(o.series[i].data=o.series[i].data.map((function(v,k){var q=c[k]||(vary?cl({"a:schemeClr":{attrs:{val:acc[k%6]}}}):void 0);return q?{value:v,itemStyle:{color:q}}:v})))}));'
         'var lg=Yn.get(e,["c:legend"]);if(lg){var lp=Yn.get(lg,["c:legendPos","attrs","val"])||"r";o.legend={show:!0,orient:"r"===lp||"l"===lp||"tr"===lp?"vertical":"horizontal"},'
         '"t"===lp?(o.legend.top="top",o.legend.left="center"):"r"===lp||"tr"===lp?(o.legend.right=8,o.legend.top="tr"===lp?"top":"middle"):"l"===lp?(o.legend.left=8,o.legend.top="middle"):(o.legend.bottom="bottom",o.legend.left="center")}else o.legend={show:!1};'
         'var ti=Yn.get(e,["c:title"]);o.title=o.title||{};if(ti){var tp=Yn.get(ti,["c:tx","c:rich","a:p"]);Array.isArray(tp)&&(tp=tp[0]);'
         'var ra=Yn.get(tp,["a:pPr","a:defRPr","attrs"])||{},rb=Yn.get(tp,["a:r","attrs"])?{}:Yn.get(tp,["a:r","a:rPr","attrs"])||{},sz=+(rb.sz||ra.sz||1400);'
         'o.title.textStyle={fontWeight:"1"===(rb.b||ra.b)?"bold":"normal",fontSize:Math.round(sz/75),color:"#595959"}}'
         'else"1"!==Yn.get(e,["c:autoTitleDeleted","attrs","val"])&&1===ser.length&&o.series[0].name?(o.title.text=o.series[0].name,o.title.textStyle={fontWeight:"normal",fontSize:19,color:"#595959"}):o.title.show=!1'
         '}catch(r){console.warn("pptx chart",r)}},')
rep('e.prototype.parseChartTitle=function(t){', CHART + 'e.prototype.parseChartTitle=function(t){')
for k in ('Bar', 'Line'):
    rep('this.options.color=this.parse%sColors(n),this.options.legend={bottom:"bottom",left:"center"}' % k,
        'this.options.color=this.parse%sColors(n),this.options.legend={bottom:"bottom",left:"center"},this.__peekChart(t,e)' % k)

# 15. 形状的线性渐变绕形状中心旋转；原库绕左上角转，角度 180–360° 时整块变成起点色（WPS 里是渐隐）
rep('h.setAttribute("gradientTransform","rotate(".concat(a.ang,")"))',
    'h.setAttribute("gradientTransform","rotate(".concat(a.ang,",0.5,0.5)"))/* Peek 补丁 */')

# 14. SVG 图片要给对 MIME，否则 data URL 是 image/* 显示不出来
rep('case"gif":e="image/gif";break;', 'case"gif":e="image/gif";break;case"svg":e="image/svg+xml";break;/* Peek 补丁 */')

# 18. 母版 txStyles / defaultTextStyle 用的段落解析 kr() 只认对齐、缩进、行距，母版 bodyStyle 里的项目符号丢了（Office 模板的正文圆点全没）
BU = ('Yn.get(t,["a:buAutoNum","attrs","type"])&&(e.buAutoNum=t["a:buAutoNum"].attrs.type),'
      'Yn.get(t,["a:buChar","attrs","char"])&&(e.buChar=t["a:buChar"].attrs.char),'
      'Yn.get(t,["a:buSzPct","attrs","val"])&&(e.buSzPct=parseInt(t["a:buSzPct"].attrs.val)/1e5),'
      'Yn.get(t,["a:spcBef","a:spcPts","attrs","val"])&&(e.spaceBefore=tr(parseInt(t["a:spcBef"]["a:spcPts"].attrs.val))),'
      'Yn.get(t,["a:spcAft","a:spcPts","attrs","val"])&&(e.spaceAfter=tr(parseInt(t["a:spcAft"]["a:spcPts"].attrs.val))),')
NONE = 't["a:buNone"]&&(e.buChar=e.buAutoNum="")/* Peek 补丁：buNone 盖掉继承的符号 */,'
rep('(e.lineHeight=parseInt(t["a:lnSpc"]["a:spcPct"].attrs.val)/1e5),e}function Lr(',
    '(e.lineHeight=parseInt(t["a:lnSpc"]["a:spcPct"].attrs.val)/1e5),/* Peek 补丁：母版项目符号 */' + BU + NONE + 'e}function Lr(')
rep('}catch(x){}}(this)/* Peek 补丁 */,', '}catch(x){}}(this)/* Peek 补丁 */,' + NONE, count=2)

# 19. 竖排：框高按 PPT 的 xfrm 定死，段落的 margin-top 一占，两个字的竖排放不下就折成两列，看着像横排；
#     改成最小高度并且 wrap="none" 时不折行。vert/vert270（整体转 90°）原库当横排处理
rep('"eaVert"===l.vert?(u.style.writingMode="vertical-rl",u.style.height=e.h+"px"):u.style.width=e.w+"px"',
    '/^(eaVert|vert|vert270|wordArtVert|mongolianVert)$/.test(l.vert)?(/* Peek 补丁：竖排 */u.style.writingMode="vert270"===l.vert?"sideways-lr":"vertical-rl",'
    '"vert"===l.vert&&(u.style.textOrientation="sideways"),"wordArtVert"===l.vert&&(u.style.textOrientation="upright"),'
    'u.style.minHeight=e.h+"px","none"===l.wrap&&(u.style.whiteSpace="nowrap")):u.style.width=e.w+"px"')

# 20. 超链接：PowerPoint/WPS 用主题的 hlink 色加下划线显示，原库不认 hlinkClick，显示成普通文字
rep('r&&(e.color=ir(r,this.node.theme,this.node));',
    'r&&(e.color=ir(r,this.node.theme,this.node));Yn.get(t,"a:hlinkClick")&&function(s){/* Peek 补丁：超链接 */e.underline=e.underline&&"none"!==e.underline?e.underline:"sng";'
    'try{e.color=ir({"a:schemeClr":{attrs:{val:"hlink"}}},s.node.theme,s.node)}catch(x){}}(this);')

# 21. 字距 spc（1/100 磅，和字号同一换算）；原库忽略
rep('return a.bold&&(l.style.fontWeight="bold"),',
    'return a.spc&&+a.spc&&(l.style.letterSpacing=parseInt(a.spc)/100*(u/(a.size||u))+"px")/* Peek 补丁：字距 */,a.bold&&(l.style.fontWeight="bold"),')

open(P, 'w', encoding='utf8', newline='').write(s)
print('patched')
