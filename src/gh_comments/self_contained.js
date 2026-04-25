document.addEventListener('DOMContentLoaded', function() {
  document.getElementById('gh-comments-export-btn').addEventListener('click', async () => {
    try {
      // Generate the a self-contained version of the current webpage
      const htmlContent = await generateSelfContainedHTML();

      // Trigger a download of the HTML content
      triggerUserDownload(htmlContent, `gh-comments-${ISSUE_ID}-export-${new Date().toISOString().slice(0,10)}.html`);
    } catch (e) {
      console.log(e);
      alert(`Error: ${e.message}`);
    }
  });
});

async function generateSelfContainedHTML() {
  // Clone current DOM into a new document
  const doc = new DOMParser().parseFromString(document.documentElement.outerHTML, 'text/html');

  // Reset all review threads expanded state to their default
  const dataExpandable = doc.querySelectorAll('[data-expandable]');
  dataExpandable.forEach(el => {
    const attrValue = el.getAttribute('data-expandable');
    el.open = (attrValue === 'true');
  });
  
  // Remove all elements with the data-to-remove-on-export attribute
  const dataElementsToRemove = doc.querySelectorAll('[data-to-remove-on-export]');
  dataElementsToRemove.forEach(el => el.remove());
  
  // Remove all elements <link rel="stylesheet"> attribute
  const linkElementsToRemove = doc.querySelectorAll('link[rel="stylesheet"]');
  linkElementsToRemove.forEach(el => el.remove());

  // Inline styles and images
  inlineStyles(doc);
  await inlineImages(doc);

  return `<!DOCTYPE html>${doc.documentElement.outerHTML}`;
}

function inlineStyles(doc) {
  const styleSheets = [...document.styleSheets];

  for (const sheet of styleSheets) {
    const rules = sheet.cssRules;
    if (!rules) continue;

    // Collect all CSS text from this stylesheet
    let cssText = '';
    for (const rule of rules) {
      cssText += rule.cssText + '\n';
    }

    // Create an inline <style> using the collected rules
    const style = doc.createElement('style');
    style.textContent = cssText;

    doc.head.appendChild(style);
  }
}

async function inlineImages(doc) {
  const imgs = Array.from(doc.querySelectorAll('img[src]'));
  await Promise.all(imgs.map(async img => {
    const src = img.getAttribute('src');
    if (src.startsWith('data:')) return;
    
    // Use the original document’s image size (not the cloned doc, as it's not render)
    const originalImg = document.querySelector(`img[src="${src}"]`);
    const cs = originalImg ? window.getComputedStyle(originalImg) : {};
    const w = Math.floor(parseFloat(cs.width) || originalImg?.width || 0) || 0;
    const h = Math.floor(parseFloat(cs.height) || originalImg?.height || 0) || 0;

    // Fetch the image, compress it and assign it to the img element as a data: URI    
    img.src = await new Promise((resolve, reject) => {
      const tempImg = new Image();
      tempImg.crossOrigin = 'anonymous';
      tempImg.onload = () => {
        const canvas = document.createElement('canvas');
        canvas.width = w || tempImg.width;
        canvas.height = h || tempImg.height;
        canvas.getContext('2d').drawImage(tempImg, 0, 0, canvas.width, canvas.height);
        resolve(canvas.toDataURL('image/png'));
      };
      tempImg.onerror = (event) => {
        // We got an error, probably a CORS rejection (happens with user images on GitHub), but we
        // can't be certain. We only get an event, not an error.
        //
        // Anyway, let's ignore this error and use the initial source.
        resolve(src);
      };
      tempImg.src = src;
    });
  }));
}

function triggerUserDownload(content, filename) {
  const blob = new Blob([content], { type: 'text/html;charset=utf-8' });
  const url = URL.createObjectURL(blob);
  
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}
